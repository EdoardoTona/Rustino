//! Webview features that make the page behave like an app rather than a browser: native paths
//! of the files dropped on the window, downloads saved where the user chooses, the browser's
//! context menu and shortcuts, the document title, cookies.
//!
//! Options are applied when the webview is built. Runtime operations go through
//! [`RustinoWindow::invoke`](crate::window::RustinoWindow::invoke), on the event loop thread.

use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tao::event_loop::EventLoopProxy;
use wry::cookie::{Cookie, SameSite};
use wry::{DragDropEvent, WebViewBuilder};

use crate::commands::RustinoCommand;
use crate::dialogs::{self, DialogParams, FileDialogKind};
use crate::invoke::Task;
use crate::state::SharedState;

/// Keeps the browser's context menu from opening where WebView2's setting doesn't exist. The
/// page still gets the `contextmenu` event, e.g. to show its own menu.
#[cfg(not(target_os = "windows"))]
const NO_CONTEXT_MENU_SCRIPT: &str =
    "addEventListener('contextmenu', (e) => e.preventDefault(), true);";

/// The page's `window.print()` opens the system print dialog, like `Print()`: WebView2 would show
/// its preview inside the page, cut by small windows, and WKWebView nothing (WebKitGTK already
/// shows the dialog). Frames without `window.ipc` keep the original.
#[cfg(any(target_os = "windows", target_os = "macos"))]
const PRINT_SCRIPT: &str = r#"(() => {
  const print = window.print;
  window.print = function () {
    if (window.ipc) window.ipc.postMessage('__rustino:print');
    else print.call(window);
  };
})();"#;

/// The message of `PRINT_SCRIPT`.
#[cfg_attr(target_os = "linux", allow(dead_code))]
const PRINT_MESSAGE: &str = "__rustino:print";

/// Keeps the messages of `PRINT_SCRIPT` from the host and opens the print dialog.
pub struct PrintFilter {
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    proxy: EventLoopProxy<RustinoCommand>,
}

impl PrintFilter {
    pub fn new(proxy: EventLoopProxy<RustinoCommand>) -> Self {
        Self { proxy }
    }

    /// Returns false for the other messages.
    pub fn handle(&self, #[allow(unused_variables)] message: &str) -> bool {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        if message == PRINT_MESSAGE {
            // After the webview event that delivered the message
            let _ = self.proxy.send_event(RustinoCommand::Invoke(Task::new(|_, webview| print(webview))));
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
        proxy: EventLoopProxy<RustinoCommand>,
    ) -> WebViewBuilder<'a> {
        let options = &self.options;
        let host = Host {
            context: context as usize,
            callbacks: self.callbacks,
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

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------
//
// The webview writes every download to a temporary file, moved to its destination when complete.
// The save dialog opens after the webview's handler returns: wry keeps the handler borrowed while
// it runs, so a second download starting while a modal dialog runs inside it would panic.

enum Destination {
    /// The save dialog is open
    Asking,
    Chosen(PathBuf),
    Canceled,
}

struct Download {
    id: u64,
    url: String,
    /// The file the webview writes, alone in its folder
    temp: PathBuf,
    /// Where the webview would have saved the file, in the Downloads folder
    suggested: PathBuf,
    destination: Destination,
    /// Whether the webview wrote the whole file, once it's done
    finished: Option<bool>,
}

#[derive(Default)]
struct Downloads {
    next_id: u64,
    list: Vec<Download>,
}

impl Downloads {
    fn add(&mut self, url: String, temp: PathBuf, suggested: PathBuf, destination: Destination) -> u64 {
        self.next_id += 1;
        self.list.push(Download {
            id: self.next_id,
            url,
            temp,
            suggested,
            destination,
            finished: None,
        });
        self.next_id
    }

    /// The webview is done with a download: returns it once its destination is known.
    fn finish(&mut self, url: &str, path: Option<&Path>, success: bool) -> Option<Download> {
        // WKWebView reports only the URL
        let index = path
            .and_then(|path| self.list.iter().position(|d| d.temp == path))
            .or_else(|| self.list.iter().position(|d| d.finished.is_none() && d.url == url))?;
        self.list[index].finished = Some(success);
        self.take_if_done(index)
    }

    /// The user chose the destination: returns the download once the webview is done with it.
    fn choose(&mut self, id: u64, destination: Option<PathBuf>) -> Option<Download> {
        let index = self.list.iter().position(|d| d.id == id)?;
        self.list[index].destination = destination.map_or(Destination::Canceled, Destination::Chosen);
        self.take_if_done(index)
    }

    fn take_if_done(&mut self, index: usize) -> Option<Download> {
        let download = &self.list[index];
        let done = download.finished.is_some() && !matches!(download.destination, Destination::Asking);
        done.then(|| self.list.remove(index))
    }
}

impl Drop for Downloads {
    /// The window closed: the webview no longer writes the files
    fn drop(&mut self) {
        for download in &self.list {
            remove_temp(&download.temp);
        }
    }
}

fn lock(downloads: &Mutex<Downloads>) -> std::sync::MutexGuard<'_, Downloads> {
    downloads.lock().unwrap_or_else(|e| e.into_inner())
}

fn download_starting(
    url: String,
    path: &mut PathBuf,
    host: Host,
    downloads: &Arc<Mutex<Downloads>>,
    proxy: &EventLoopProxy<RustinoCommand>,
) -> bool {
    let suggested = path.clone();
    let mut destination = Destination::Asking;
    if let Some(cb) = host.callbacks.on_download_starting {
        let (Ok(c_url), Ok(c_suggested)) = (
            CString::new(url.as_str()),
            CString::new(suggested.to_string_lossy().as_bytes()),
        ) else {
            return false;
        };
        let mut response = DownloadResponse::default();
        let allowed = crate::invoke::webview_event(|| unsafe {
            cb(host.context as *mut c_void, c_url.as_ptr(), c_suggested.as_ptr(), &mut response)
        });
        if allowed == 0 {
            return false;
        }
        if let Some(chosen) = response.destination.filter(|d| !d.is_empty()) {
            destination = Destination::Chosen(PathBuf::from(chosen));
        }
    }
    let Some(temp) = temp_download_path(&suggested) else {
        return false;
    };
    let ask = matches!(destination, Destination::Asking);
    let id = lock(downloads).add(url, temp.clone(), suggested, destination);
    if ask {
        let downloads = Arc::clone(downloads);
        let proxy_for_task = proxy.clone();
        let _ = proxy.send_event(RustinoCommand::Invoke(Task::new(move |window, _| {
            ask_destination(id, window, &downloads, host, proxy_for_task)
        })));
    }
    *path = temp;
    true
}

/// Shows the save dialog for a download, after the webview's handler returned.
fn ask_destination(
    id: u64,
    window: &tao::window::Window,
    downloads: &Mutex<Downloads>,
    host: Host,
    proxy: EventLoopProxy<RustinoCommand>,
) {
    let suggested = lock(downloads).list.iter().find(|d| d.id == id).map(|d| d.suggested.clone());
    let Some(suggested) = suggested else {
        return;
    };
    let params = DialogParams {
        title: None,
        default_path: Some(suggested.to_string_lossy().into_owned()),
        filters: Vec::new(),
    };
    // Not locked: other downloads can start and finish while the dialog is open
    let chosen = dialogs::show_file_dialog(FileDialogKind::Save, &params, Some(window))
        .and_then(|paths| paths.into_iter().next())
        .map(PathBuf::from);
    let done = lock(downloads).choose(id, chosen);
    if let Some(download) = done {
        complete(download, host, proxy);
    }
}

/// Moves the file to its destination and reports the download to the host.
fn complete(download: Download, host: Host, proxy: EventLoopProxy<RustinoCommand>) {
    // Across volumes the move copies the file: not on the event loop thread
    std::thread::spawn(move || {
        let path = match (&download.destination, download.finished) {
            (Destination::Chosen(destination), Some(true)) => {
                move_file(&download.temp, destination).ok().map(|_| destination.clone())
            }
            _ => None,
        };
        remove_temp(&download.temp);
        let url = download.url;
        let _ = proxy.send_event(RustinoCommand::Invoke(Task::new(move |_, _| {
            host.download_completed(&url, path.as_deref())
        })));
    });
}

/// Name of the temporary download folders: `rustino-download-<process id>-<random>`.
const TEMP_FOLDER_PREFIX: &str = "rustino-download-";

/// A new file in its own temporary folder, with the suggested name.
fn temp_download_path(suggested: &Path) -> Option<PathBuf> {
    static CLEANUP: std::sync::Once = std::sync::Once::new();
    let temp_dir = std::env::temp_dir();
    CLEANUP.call_once(|| {
        let temp_dir = temp_dir.clone();
        std::thread::spawn(move || remove_stale_temp_folders(&temp_dir));
    });
    let folder = create_private_folder(&temp_dir)?;
    let name = suggested.file_name().map_or_else(|| "download".into(), |n| n.to_os_string());
    Some(folder.join(name))
}

/// Creates a new folder in `parent` (often shared with the other users, like `/tmp`) that only
/// this user can open, with an unpredictable name. The creation fails if the path exists, so
/// nobody else can prepare the folder or a link in its place.
fn create_private_folder(parent: &Path) -> Option<PathBuf> {
    for _ in 0..16 {
        let name = format!("{TEMP_FOLDER_PREFIX}{}-{:016x}{:016x}", std::process::id(), random_u64(), random_u64());
        let folder = parent.join(name);
        #[allow(unused_mut)]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&folder) {
            Ok(()) => return Some(folder),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// 64 random bits: SipHash with the random keys of `RandomState`, over a counter and the time.
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    hasher.write_u128(now.map_or(0, |d| d.as_nanos()));
    hasher.finish()
}

/// The process that created a temporary download folder, from its name.
fn temp_folder_process(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(TEMP_FOLDER_PREFIX)?;
    let (pid, random) = rest.split_once('-')?;
    (random.len() == 32 && random.bytes().all(|b| b.is_ascii_hexdigit())).then_some(())?;
    pid.parse().ok()
}

/// Removes the temporary download folders of the processes that ended without removing them
/// (e.g. crashed). Only the folders of this user, never through a link.
fn remove_stale_temp_folders(temp_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(temp_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(temp_folder_process) else {
            continue;
        };
        if pid == std::process::id() || process_alive(pid) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        #[cfg(unix)]
        let mine = std::os::unix::fs::MetadataExt::uid(&metadata) == unsafe { unix::getuid() };
        // The temporary folder of Windows is the user's
        #[cfg(not(unix))]
        let mine = true;
        if metadata.is_dir() && mine {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(unix)]
mod unix {
    unsafe extern "C" {
        pub fn getuid() -> u32;
        pub fn kill(pid: i32, signal: i32) -> i32;
    }
}

/// Whether a process runs. In doubt (e.g. no permission to check) it does.
fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // Signal 0 only checks the process: EPERM means that it runs as another user
        const ESRCH: i32 = 3;
        let result = unsafe { unix::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, STILL_ACTIVE};
        use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(process) => {
                    let mut code = 0u32;
                    let exited = GetExitCodeProcess(process, &mut code).is_ok() && code != STILL_ACTIVE.0 as u32;
                    let _ = CloseHandle(process);
                    !exited
                }
                Err(e) => e.code() != ERROR_INVALID_PARAMETER.to_hresult(),
            }
        }
    }
}

fn remove_temp(temp: &Path) {
    if let Some(folder) = temp.parent() {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// Replaces an existing file: the save dialog already asked.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ()))
}

// ---------------------------------------------------------------------------
// Cookies
// ---------------------------------------------------------------------------

/// A cookie as the host sees it (JSON, camelCase).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CookieData {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    /// Unix time in seconds; none for session cookies
    #[serde(default)]
    pub expires: Option<i64>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// "Strict", "Lax" or "None"
    #[serde(default)]
    pub same_site: Option<String>,
}

impl From<&Cookie<'_>> for CookieData {
    fn from(cookie: &Cookie<'_>) -> Self {
        Self {
            name: cookie.name().to_string(),
            value: cookie.value().to_string(),
            domain: cookie.domain().map(str::to_string),
            path: cookie.path().map(str::to_string),
            expires: cookie.expires_datetime().map(|t| t.unix_timestamp()),
            secure: cookie.secure().unwrap_or(false),
            http_only: cookie.http_only().unwrap_or(false),
            same_site: cookie.same_site().map(|s| s.to_string()),
        }
    }
}

impl CookieData {
    pub fn to_cookie(&self) -> Cookie<'static> {
        let mut builder = Cookie::build((self.name.clone(), self.value.clone()))
            .secure(self.secure)
            .http_only(self.http_only);
        if let Some(domain) = &self.domain {
            builder = builder.domain(domain.clone());
        }
        if let Some(path) = &self.path {
            builder = builder.path(path.clone());
        }
        if let Some(expires) = self.expires.and_then(|t| wry::cookie::time::OffsetDateTime::from_unix_timestamp(t).ok()) {
            builder = builder.expires(expires);
        }
        let same_site = match self.same_site.as_deref() {
            Some(s) if s.eq_ignore_ascii_case("strict") => Some(SameSite::Strict),
            Some(s) if s.eq_ignore_ascii_case("lax") => Some(SameSite::Lax),
            Some(s) if s.eq_ignore_ascii_case("none") => Some(SameSite::None),
            _ => None,
        };
        if let Some(same_site) = same_site {
            builder = builder.same_site(same_site);
        }
        builder.build()
    }
}

/// JSON array of the webview's cookies, all of them or those sent to `url`.
pub fn cookies_json(webview: &wry::WebView, url: Option<&str>) -> Option<String> {
    let cookies = match url {
        Some(url) => webview.cookies_for_url(url),
        None => webview.cookies(),
    }
    .ok()?;
    let data: Vec<CookieData> = cookies.iter().map(CookieData::from).collect();
    serde_json::to_string(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download(downloads: &mut Downloads, url: &str, destination: Destination) -> (u64, PathBuf) {
        let temp = PathBuf::from(format!("/tmp/rustino-test/{url}/file"));
        let id = downloads.add(url.into(), temp.clone(), PathBuf::from("/downloads/file"), destination);
        (id, temp)
    }

    #[test]
    fn download_waits_for_the_dialog_and_the_webview() {
        let mut downloads = Downloads::default();
        let (id, temp) = download(&mut downloads, "a", Destination::Asking);
        assert!(downloads.finish("a", Some(&temp), true).is_none(), "the dialog is still open");
        let done = downloads.choose(id, Some(PathBuf::from("/chosen"))).unwrap();
        assert!(matches!(done.destination, Destination::Chosen(ref p) if p == Path::new("/chosen")));
        assert_eq!(done.finished, Some(true));
        assert!(downloads.list.is_empty());
    }

    #[test]
    fn download_chosen_before_it_finishes() {
        let mut downloads = Downloads::default();
        let (id, _) = download(&mut downloads, "a", Destination::Asking);
        assert!(downloads.choose(id, None).is_none(), "still downloading");
        // WKWebView reports only the URL
        let done = downloads.finish("a", None, true).unwrap();
        assert!(matches!(done.destination, Destination::Canceled));
    }

    #[test]
    fn download_with_host_destination_is_done_when_finished() {
        let mut downloads = Downloads::default();
        download(&mut downloads, "a", Destination::Chosen("/x".into()));
        let (_, temp_b) = download(&mut downloads, "b", Destination::Chosen("/y".into()));
        let done = downloads.finish("ignored", Some(&temp_b), false).unwrap();
        assert_eq!(done.url, "b");
        assert_eq!(done.finished, Some(false));
        assert!(downloads.finish("unknown", None, true).is_none());
        assert_eq!(downloads.list.len(), 1);
    }

    #[test]
    fn temp_download_path_is_alone_in_its_folder() {
        let a = temp_download_path(Path::new("/downloads/report.pdf")).unwrap();
        let b = temp_download_path(Path::new("/downloads/report.pdf")).unwrap();
        assert_eq!(a.file_name().unwrap(), "report.pdf");
        assert_ne!(a.parent(), b.parent());
        assert!(!a.exists(), "the webview creates the file");
        remove_temp(&a);
        remove_temp(&b);
        assert!(!a.parent().unwrap().exists());
    }

    /// A folder of the test's own, under the system's temporary folder
    fn test_parent(name: &str) -> PathBuf {
        let parent = std::env::temp_dir().join(format!("rustino-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir(&parent).unwrap();
        parent
    }

    #[test]
    fn private_folders_are_new_and_unpredictable() {
        let parent = test_parent("private");
        let a = create_private_folder(&parent).unwrap();
        let b = create_private_folder(&parent).unwrap();
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_str().unwrap();
        assert_eq!(temp_folder_process(name), Some(std::process::id()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn temp_folder_names() {
        let random = "0123456789abcdef0123456789ABCDEF";
        assert_eq!(temp_folder_process(&format!("rustino-download-42-{random}")), Some(42));
        assert_eq!(temp_folder_process("rustino-download-42-1"), None);
        assert_eq!(temp_folder_process(&format!("rustino-download-x-{random}")), None);
        assert_eq!(temp_folder_process(&format!("other-42-{random}")), None);
        assert_eq!(temp_folder_process(&format!("rustino-download-42-{}", "g".repeat(32))), None);
    }

    #[test]
    fn stale_folders_of_ended_processes_are_removed() {
        let parent = test_parent("stale");
        let random = "0123456789abcdef0123456789abcdef";
        // No process has this id: the limits are 99999 on macOS, 2^22 on Linux, and Windows ids
        // are multiples of 4
        let stale = parent.join(format!("rustino-download-99999999-{random}"));
        let own = parent.join(format!("rustino-download-{}-{random}", std::process::id()));
        let other = parent.join("rustino-download-other");
        for folder in [&stale, &own, &other] {
            std::fs::create_dir(folder).unwrap();
            std::fs::write(folder.join("file"), "x").unwrap();
        }
        remove_stale_temp_folders(&parent);
        assert!(!stale.exists());
        assert!(own.exists() && other.exists());
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn running_processes_are_alive() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(99999999));
    }

    #[test]
    fn move_file_replaces_the_destination() {
        let from = temp_download_path(Path::new("from.txt")).unwrap();
        let to = temp_download_path(Path::new("to.txt")).unwrap();
        std::fs::write(&from, "new").unwrap();
        std::fs::write(&to, "old").unwrap();
        move_file(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "new");
        assert!(!from.exists());
        remove_temp(&from);
        remove_temp(&to);
    }

    #[test]
    fn cookie_round_trip() {
        let json = r#"{"name":"session","value":"abc","domain":"example.com","path":"/","expires":1893456000,"secure":true,"httpOnly":true,"sameSite":"Lax"}"#;
        let data: CookieData = serde_json::from_str(json).unwrap();
        let cookie = data.to_cookie();
        assert_eq!(cookie.name(), "session");
        assert_eq!(cookie.domain(), Some("example.com"));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.expires_datetime().unwrap().unix_timestamp(), 1893456000);
        assert_eq!(CookieData::from(&cookie), data);
    }

    #[test]
    fn cookie_defaults() {
        let data: CookieData = serde_json::from_str(r#"{"name":"a","value":"b"}"#).unwrap();
        let cookie = data.to_cookie();
        assert_eq!(cookie.secure(), Some(false));
        assert!(cookie.expires_datetime().is_none());
        assert!(cookie.same_site().is_none());
    }
}
