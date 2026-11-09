//! Webview features that make the page behave like an app rather than a browser: native paths
//! of the files dropped on the window, downloads saved where the user chooses, the browser's
//! context menu and shortcuts, the document title, cookies.
//!
//! Options are applied when the webview is built. Runtime operations go through
//! [`RustinoWindow::invoke`](crate::window::RustinoWindow::invoke), on the event loop thread.

mod cookies;
mod downloads;
mod exports;

use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use wry::{DragDropEvent, WebViewBuilder};

use crate::config::Logger;
use crate::state::SharedState;
use crate::window::CommandProxy;

pub use cookies::{CookieData, cookies_json};
use downloads::{Downloads, complete, download_starting, lock};

/// Keeps the browser's context menu from opening where WebView2's setting doesn't exist. The
/// page still gets the `contextmenu` event, e.g. to show its own menu.
#[cfg(not(target_os = "windows"))]
const NO_CONTEXT_MENU_SCRIPT: &str =
    "addEventListener('contextmenu', (e) => e.preventDefault(), true);";

/// The page's `window.print()` opens the system print dialog, like `Print()`: WebView2 would show
/// its preview inside the page, cut by small windows, and WKWebView nothing (WebKitGTK already
/// shows the dialog). Frames without `window.ipc` keep the original.
#[cfg(any(target_os = "windows", target_os = "macos"))]
const PRINT_SCRIPT: &str = include_str!("scripts/print.js");

/// The message of `PRINT_SCRIPT`.
#[cfg_attr(target_os = "linux", allow(dead_code))]
const PRINT_MESSAGE: &str = "__rustino:print";

/// Keeps the messages of `PRINT_SCRIPT` from the host and opens the print dialog.
pub struct PrintFilter {
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    proxy: CommandProxy,
}

impl PrintFilter {
    pub fn new(proxy: CommandProxy) -> Self {
        Self { proxy }
    }

    /// Returns false for the other messages.
    pub fn handle(&self, #[allow(unused_variables)] message: &str) -> bool {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        if message == PRINT_MESSAGE {
            // After the webview event that delivered the message
            let task = crate::invoke::Task::new(|_, webview| print(webview));
            let _ = self.proxy.send_event(crate::commands::RustinoCommand::Invoke(task));
            return true;
        }
        false
    }
}

/// Options applied when the webview is built.
pub struct WebViewExtOptions {
    pub context_menu: bool,
    /// Windows: F5, Ctrl+R, Ctrl+F, Ctrl+P and the other shortcuts of the browser
    pub browser_accelerator_keys: bool,
    /// Windows: the overlay scroll bars of Windows 11
    pub fluent_overlay_scroll_bars: bool,
    /// macOS: the click that activates the window also reaches the page
    pub accept_first_mouse: bool,
    pub back_forward_gestures: bool,
    /// Off by default: on Windows the webview no longer gets the drops, so the page's HTML5
    /// drag and drop stops working
    pub file_drop: bool,
}

impl Default for WebViewExtOptions {
    fn default() -> Self {
        Self {
            context_menu: true,
            browser_accelerator_keys: true,
            fluent_overlay_scroll_bars: false,
            accept_first_mouse: false,
            back_forward_gestures: false,
            file_drop: false,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct WebViewExtCallbacks {
    /// (context, kind, paths separated by '\n', x, y): kind 0 enter, 1 over, 2 drop, 3 leave;
    /// position in logical pixels from the top-left corner of the webview
    pub on_file_drop: Option<unsafe extern "C" fn(*mut c_void, i32, *const c_char, i32, i32)>,
    /// (context, title)
    pub on_document_title_changed: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    /// (context, URL, suggested path, response) -> 0 cancels the download. The host can set the
    /// destination with `rustino_set_download_destination`; without it the user chooses one
    pub on_download_starting:
        Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut DownloadResponse) -> i32>,
    /// (context, URL, path or null, success)
    pub on_download_completed: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, i32)>,
}

/// Filled by the host while `on_download_starting` runs.
#[derive(Default)]
pub struct DownloadResponse {
    pub destination: Option<String>,
}

/// Configuration of the webview features until the window runs.
#[derive(Default)]
pub struct WebViewExt {
    pub options: WebViewExtOptions,
    pub callbacks: WebViewExtCallbacks,
}

impl WebViewExt {
    pub fn configure<'a>(
        &self,
        builder: WebViewBuilder<'a>,
        context: *mut c_void,
        state: &Arc<SharedState>,
        proxy: CommandProxy,
        logger: Logger,
    ) -> WebViewBuilder<'a> {
        let options = &self.options;
        let host = Host {
            context: context as usize,
            callbacks: self.callbacks,
            logger,
        };

        #[cfg(target_os = "windows")]
        let builder = {
            use wry::{ScrollBarStyle, WebViewBuilderExtWindows};
            builder
                .with_default_context_menus(options.context_menu)
                .with_browser_accelerator_keys(options.browser_accelerator_keys)
                .with_scroll_bar_style(if options.fluent_overlay_scroll_bars {
                    ScrollBarStyle::FluentOverlay
                } else {
                    ScrollBarStyle::Default
                })
        };
        #[cfg(not(target_os = "windows"))]
        let builder = if options.context_menu {
            builder
        } else {
            builder.with_initialization_script_for_main_only(NO_CONTEXT_MENU_SCRIPT, false)
        };

        #[cfg(any(target_os = "windows", target_os = "macos"))]
        let builder = builder.with_initialization_script_for_main_only(PRINT_SCRIPT, false);

        let mut builder = builder
            .with_accept_first_mouse(options.accept_first_mouse)
            .with_back_forward_navigation_gestures(options.back_forward_gestures);

        if let Some(cb) = self.callbacks.on_document_title_changed {
            builder = builder.with_document_title_changed_handler(move |title| {
                if let Ok(title) = CString::new(title) {
                    crate::invoke::webview_event(|| unsafe { cb(context, title.as_ptr()) });
                }
            });
        }

        if options.file_drop
            && let Some(cb) = self.callbacks.on_file_drop
        {
            builder = builder.with_drag_drop_handler(file_drop_handler(cb, context, Arc::clone(state)));
        }

        // Without a handler WKWebView doesn't download at all
        let downloads = Arc::new(Mutex::new(Downloads::default()));
        let started = {
            let downloads = Arc::clone(&downloads);
            let proxy = proxy.clone();
            move |url: String, path: &mut PathBuf| download_starting(url, path, host, &downloads, &proxy)
        };
        builder
            .with_download_started_handler(started)
            .with_download_completed_handler(move |url, path, success| {
                let done = lock(&downloads).finish(&url, path.as_deref(), success);
                if let Some(download) = done {
                    complete(download, host, proxy.clone());
                }
            })
    }
}

/// WebView2 asks before a page downloads a second file without a click: the host already decides
/// on every download.
#[cfg(target_os = "windows")]
pub fn allow_multiple_downloads(webview: &wry::WebView) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    };
    use webview2_com::PermissionRequestedEventHandler;
    use wry::WebViewExtWindows;

    let handler = PermissionRequestedEventHandler::create(Box::new(|_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
        unsafe {
            args.PermissionKind(&mut kind)?;
            if kind == COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe {
        let _ = webview.webview().add_PermissionRequested(&handler, &mut token);
    }
}

/// The system print dialog: WebView2's own preview is drawn in the page, and a small window cuts it.
pub fn print(webview: &wry::WebView) {
    #[cfg(target_os = "windows")]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM, ICoreWebView2_16};
        use windows::core::Interface;
        use wry::WebViewExtWindows;
        // WebView2 Runtime 1.0.1518 or later
        if let Ok(webview) = webview.webview().cast::<ICoreWebView2_16>() {
            unsafe {
                let _ = webview.ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM);
            }
            return;
        }
    }
    let _ = webview.print();
}

/// The web inspector. On macOS it needs the `devtools` feature (a private WebKit API).
pub fn open_devtools(#[allow(unused_variables)] webview: &wry::WebView) {
    #[cfg(any(not(target_os = "macos"), debug_assertions, feature = "devtools"))]
    webview.open_devtools();
}

/// Not supported on Windows.
pub fn close_devtools(#[allow(unused_variables)] webview: &wry::WebView) {
    #[cfg(any(not(target_os = "macos"), debug_assertions, feature = "devtools"))]
    webview.close_devtools();
}

/// Lets Safari's Develop menu inspect the page (a public API of macOS 13.3). With the `devtools`
/// feature wry does it too, next to the in-app inspector.
#[cfg(target_os = "macos")]
pub fn make_inspectable(webview: &wry::WebView) {
    use objc2::runtime::AnyObject;
    use wry::WebViewExtMacOS;
    let webview = webview.webview();
    let webview: &AnyObject = &webview;
    unsafe {
        let supported: bool = objc2::msg_send![webview, respondsToSelector: objc2::sel!(setInspectable:)];
        if supported {
            let _: () = objc2::msg_send![webview, setInspectable: true];
        }
    }
}

/// The host callbacks, for the code that runs later on the event loop thread.
#[derive(Clone, Copy)]
struct Host {
    context: usize,
    callbacks: WebViewExtCallbacks,
    logger: Logger,
}

impl Host {
    fn download_completed(&self, url: &str, path: Option<&Path>) {
        let Some(cb) = self.callbacks.on_download_completed else {
            return;
        };
        let Ok(url) = CString::new(url) else {
            return;
        };
        let path = path.and_then(|p| CString::new(p.to_string_lossy().as_bytes()).ok());
        let path_ptr = path.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
        unsafe { cb(self.context as *mut c_void, url.as_ptr(), path_ptr, i32::from(path.is_some())) };
    }
}

// ---------------------------------------------------------------------------
// File drop
// ---------------------------------------------------------------------------

fn file_drop_handler(
    cb: unsafe extern "C" fn(*mut c_void, i32, *const c_char, i32, i32),
    context: *mut c_void,
    state: Arc<SharedState>,
) -> impl Fn(DragDropEvent) -> bool {
    // Whether the drag carries files: other drags stay with the page
    let dragging_files = Cell::new(false);
    move |event| {
        let (kind, paths, (x, y)) = match event {
            DragDropEvent::Enter { paths, position } => {
                dragging_files.set(!paths.is_empty());
                (0, paths, position)
            }
            DragDropEvent::Over { position } => (1, Vec::new(), position),
            DragDropEvent::Drop { paths, position } => (2, paths, position),
            DragDropEvent::Leave => (3, Vec::new(), (0, 0)),
            _ => return false,
        };
        if !dragging_files.get() {
            return false;
        }
        if kind >= 2 {
            dragging_files.set(false);
        }
        // WebView2 reports physical pixels, WKWebView and WebKitGTK logical ones
        #[cfg(target_os = "windows")]
        let (x, y) = {
            let scale = state.load_scale_factor();
            ((x as f64 / scale).round() as i32, (y as f64 / scale).round() as i32)
        };
        #[cfg(not(target_os = "windows"))]
        let _ = &state;
        let paths: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if let Ok(paths) = CString::new(paths.join("\n")) {
            unsafe { cb(context, kind, paths.as_ptr(), x, y) };
        }
        // The page doesn't get the files, and the webview doesn't open them
        true
    }
}
