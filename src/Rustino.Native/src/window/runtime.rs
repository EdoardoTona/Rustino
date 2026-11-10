//! A window of the event loop, with its webview: builds them from the configuration and handles
//! their events.

use std::ffi::CString;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};

use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::EventLoopWindowTarget;
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use crate::callbacks::RustinoCallbacks;
use crate::commands::RustinoCommand;
use crate::config::WindowConfig;
use crate::custom_scheme::handle_custom_scheme;
use crate::icon;
use crate::menu;
use crate::monitors::update_monitor_cache;
use crate::state::SharedState;
use crate::tray::Tray;

use super::Started;
use super::dispatch::dispatch_command;
use super::event_loop::{CommandProxy, EventLoopRegistration, HostRequest, LoopProxy, Wake, register_event_loop};

/// A window of the event loop, with what its events need.
pub(super) struct WindowRuntime {
    pub(super) key: usize,
    pub(super) proxy: CommandProxy,
    callbacks: RustinoCallbacks,
    pub(super) config: WindowConfig,
    state: Arc<SharedState>,
    ext: crate::window_ext::WindowExtRuntime,
    menu_items: menu::MenuItems,
    current_menu: Option<muda::Menu>,
    tray: Option<Tray>,
    _menu_events: EventLoopRegistration,
    _global_shortcuts: crate::global_shortcuts::Owner,
    /// Keeps the window and the webview reachable from `invoke`
    _running: crate::invoke::RunningGuard,
    // Dropped before the window
    _parent: Option<crate::parent::Child>,
    webview: Rc<wry::WebView>,
    pub(super) window: Rc<tao::window::Window>,
    _web_context: Option<wry::WebContext>,
    /// Windows that joined the event loop: disconnects once the window closes, which ends its `run`
    closed_reply: Option<mpsc::Sender<Result<(), String>>>,
}

impl WindowRuntime {
    /// Builds a window that joins the running event loop, and tells its `run` the outcome.
    pub(super) fn host(
        request: HostRequest,
        target: &EventLoopWindowTarget<Wake>,
        proxy: &LoopProxy,
    ) -> Option<Self> {
        let HostRequest {
            key,
            state,
            started,
            reply,
        } = request;
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::new(key, &state, started, target, proxy, false)
        }))
        .unwrap_or_else(|panic| Err(crate::util::panic_message(panic.as_ref())));
        match built {
            Ok(mut window) => {
                // The event loop started before the window
                window.handle(Event::NewEvents(StartCause::Init), target);
                let _ = reply.send(Ok(()));
                window.closed_reply = Some(reply);
                Some(window)
            }
            Err(message) => {
                let _ = reply.send(Err(message));
                None
            }
        }
    }

    /// Builds the window and its webview. `main`: the window that runs the event loop.
    pub(super) fn new(
        key: usize,
        state: &Arc<SharedState>,
        started: Started,
        target: &EventLoopWindowTarget<Wake>,
        proxy: &LoopProxy,
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] main: bool,
    ) -> Result<Self, String> {
        let Started {
            config,
            callbacks,
            ext,
            webview_ext,
        } = started;

        warn_unsupported_settings(&config);
        let proxy = CommandProxy {
            key,
            proxy: proxy.clone(),
        };

        // --- Build window ---
        let mut window_builder = WindowBuilder::new()
            .with_title(&config.title)
            .with_resizable(config.resizable)
            .with_always_on_top(config.topmost)
            .with_decorations(config.decorations)
            .with_visible(config.visible)
            .with_maximized(config.maximized);

        if config.transparent {
            window_builder = window_builder.with_transparent(true);
        }

        if !config.use_os_default_size {
            window_builder =
                window_builder.with_inner_size(PhysicalSize::new(config.width, config.height));
        }

        if let Some((w, h)) = config.min_size {
            window_builder = window_builder.with_min_inner_size(PhysicalSize::new(w, h));
        }

        if let Some((w, h)) = config.max_size {
            window_builder = window_builder.with_max_inner_size(PhysicalSize::new(w, h));
        }

        if let Some(ref icon_path) = config.icon_file
            && let Some(ico) = icon::load_icon(icon_path)
        {
            window_builder = window_builder.with_window_icon(Some(ico));
        }

        if let Some(color) = config.background_color {
            window_builder = window_builder.with_background_color(color);
        }

        window_builder = ext.configure_window(window_builder);

        // The parent runs in this event loop
        let parent = config.parent.and_then(|key| {
            let parent = crate::invoke::running_window(key);
            if parent.is_none() {
                config.logger().warning("The parent window doesn't run: the window opens without it.");
            }
            parent
        });
        if let Some(ref parent) = parent {
            window_builder = crate::parent::configure(window_builder, parent, config.modal);
        }

        let window = window_builder
            .build(target)
            .map_err(|e| format!("Failed to create the window: {e}"))?;

        if config.center {
            center_window(&window);
        }

        if let Some((x, y)) = config.position {
            window.set_outer_position(PhysicalPosition::new(x, y));
        }

        if config.fullscreen {
            window.set_fullscreen(Some(tao::window::Fullscreen::Borderless(None)));
        }

        // --- Build webview ---
        let mut web_context = config
            .user_data_folder
            .as_ref()
            .map(|p| wry::WebContext::new(Some(std::path::PathBuf::from(p))));

        let mut webview_builder = match web_context {
            Some(ref mut ctx) => WebViewBuilder::new_with_web_context(ctx),
            None => WebViewBuilder::new(),
        };

        if config.devtools_enabled {
            webview_builder = webview_builder.with_devtools(true);
        }

        if config.clipboard_enabled {
            webview_builder = webview_builder.with_clipboard(true);
        }

        if config.transparent {
            webview_builder = webview_builder.with_transparent(true);
        }

        if let Some(color) = config.background_color {
            webview_builder = webview_builder.with_background_color(color);
        }

        if let Some(ref ua) = config.user_agent {
            webview_builder = webview_builder.with_user_agent(ua);
        }

        webview_builder = webview_builder.with_autoplay(config.media_autoplay);

        #[cfg(target_os = "windows")]
        if let Some(args) = webview2_browser_args(&config) {
            use wry::WebViewBuilderExtWindows;
            webview_builder = webview_builder.with_additional_browser_args(args);
        }

        if config.zoom_hotkeys {
            webview_builder = webview_builder.with_hotkeys_zoom(true);
        }

        webview_builder = ext.configure_webview(webview_builder);
        let ipc_filter = ext.ipc_filter(proxy.clone());
        let print_filter = crate::webview_ext::PrintFilter::new(proxy.clone());
        webview_builder = webview_ext.configure(
            webview_builder,
            callbacks.context,
            state,
            proxy.clone(),
            config.logger(),
        );

        for script in &config.initialization_scripts {
            webview_builder = webview_builder.with_initialization_script(script);
        }

        // IPC handler: JS → Rust, with the URL of the page that sent the message
        let ctx = callbacks.context;
        if let Some(cb) = callbacks.on_web_message {
            #[cfg(target_os = "windows")]
            let schemes = config.custom_schemes.clone();
            webview_builder = webview_builder.with_ipc_handler(move |req: wry::http::Request<String>| {
                if print_filter.handle(req.body()) || ipc_filter.handle(req.body()) {
                    return;
                }
                let source = req.uri().to_string();
                #[cfg(target_os = "windows")]
                let source = crate::custom_scheme::revert_custom_scheme_workaround(&source, &schemes);
                if let (Ok(message), Ok(source)) = (CString::new(req.into_body()), CString::new(source)) {
                    crate::invoke::webview_event(|| unsafe { cb(ctx, message.as_ptr(), source.as_ptr()) });
                }
            });
        } else {
            webview_builder = webview_builder.with_ipc_handler(move |req: wry::http::Request<String>| {
                let _ = print_filter.handle(req.body()) || ipc_filter.handle(req.body());
            });
        }

        // Navigation handler
        if let Some(cb) = callbacks.on_navigation {
            webview_builder = webview_builder.with_navigation_handler(move |url| {
                match CString::new(url) {
                    Ok(cstr) => crate::invoke::webview_event(|| unsafe { cb(ctx, cstr.as_ptr()) == 0 }),
                    Err(_) => true,
                }
            });

            webview_builder = webview_builder.with_new_window_req_handler(move |url, _features| {
                handle_new_window_req(url, ctx, cb)
            });
        }

        // Custom scheme handlers: webview request → host response
        if let Some(cb) = callbacks.on_custom_scheme {
            // wry wants a Send + Sync handler, but it runs it on the event loop thread, like the
            // other callbacks: the context crosses as an address
            let ctx = ctx as usize;
            for scheme in &config.custom_schemes {
                webview_builder = webview_builder.with_custom_protocol(scheme.clone(), move |_id, request| {
                    crate::invoke::webview_event(|| {
                        handle_custom_scheme(request.uri().to_string(), ctx as *mut std::ffi::c_void, cb)
                    })
                });
            }
        }

        // Page load handler
        if let Some(cb) = callbacks.on_page_load {
            webview_builder =
                webview_builder.with_on_page_load_handler(move |event, url| {
                    let event_code = match event {
                        wry::PageLoadEvent::Started => 0,
                        wry::PageLoadEvent::Finished => 1,
                    };
                    if let Ok(cstr) = CString::new(url) {
                        crate::invoke::webview_event(|| unsafe { cb(ctx, event_code, cstr.as_ptr()) });
                    }
                });
        }

        if let Some(ref url) = config.start_url {
            webview_builder = webview_builder.with_url(url);
        } else if let Some(ref html) = config.start_html {
            webview_builder = webview_builder.with_html(html);
        }

        // On Linux the webview goes in tao's GTK box, next to the menu bar: `build` only
        // supports X11 and would draw over the menu bar
        #[cfg(target_os = "linux")]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            let vbox = window.default_vbox().ok_or("Failed to create the webview: the window has no GTK box")?;
            webview_builder.build_gtk(vbox)
        };
        #[cfg(not(target_os = "linux"))]
        let webview = webview_builder.build(&window);
        let webview = webview.map_err(|e| format!("Failed to create the webview: {e}"))?;

        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowExtWindows;
            crate::menu::accelerators::attach_webview(&webview, window.hwnd());
            crate::webview_ext::allow_multiple_downloads(&webview);
        }

        #[cfg(target_os = "macos")]
        if config.devtools_enabled {
            crate::webview_ext::make_inspectable(&webview);
        }

        // Reachable from the callbacks, which run on this thread
        let (window, webview) = (Rc::new(window), Rc::new(webview));
        let running = crate::invoke::register_running(key, &window, &webview);
        let parent = parent.map(|parent| crate::parent::attach(&window, &parent, config.modal));
        for accelerator in &config.global_shortcuts {
            if let Err(message) = crate::global_shortcuts::register(key, accelerator) {
                config.logger().warning(&message);
            }
        }

        // Initialize shared state from actual window
        let size = window.inner_size();
        let pos = window
            .outer_position()
            .unwrap_or(PhysicalPosition::new(0, 0));
        state.store_size(size.width, size.height);
        state.store_position(pos.x, pos.y);
        state
            .is_maximized
            .store(window.is_maximized(), Ordering::Release);
        state
            .is_fullscreen
            .store(config.fullscreen, Ordering::Release);
        state
            .is_visible
            .store(config.visible, Ordering::Release);

        update_monitor_cache(&window, state);
        let ext = ext.start(&window, state, config.decorations, config.topmost, callbacks.context, proxy.clone());

        // Menu and tray events reach this window through its event loop
        let menu_events = register_event_loop(proxy.clone(), &window);

        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut menu_items = menu::MenuItems::default();
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut current_menu: Option<muda::Menu> = None;
        // The menu bar of macOS belongs to the app: the other windows keep it
        #[cfg(target_os = "macos")]
        if main {
            let about = menu::about_metadata(&config);
            if let Some(built) = menu::build_menu(menu::DEFAULT_MACOS_MENU, &about) {
                menu::set_menu_bar(built, &window, &config, &about, &mut current_menu, &mut menu_items);
            }
        }

        Ok(Self {
            key,
            proxy,
            callbacks,
            config,
            state: Arc::clone(state),
            ext,
            menu_items,
            current_menu,
            tray: None,
            _menu_events: menu_events,
            _global_shortcuts: crate::global_shortcuts::Owner(key),
            _running: running,
            _parent: parent,
            webview,
            window,
            _web_context: web_context,
            closed_reply: None,
        })
    }

    /// Handles an event of this window. Returns true when the window closes.
    pub(super) fn handle(&mut self, event: Event<'_, RustinoCommand>, target: &EventLoopWindowTarget<Wake>) -> bool {
        if let Event::UserEvent(RustinoCommand::GlobalShortcutPressed(id)) = event {
            if let (Some(cb), Some(accelerator)) = (
                self.callbacks.on_global_shortcut,
                crate::global_shortcuts::accelerator(self.key, id),
            ) && let Ok(accelerator) = CString::new(accelerator)
            {
                unsafe { cb(self.callbacks.context, accelerator.as_ptr()) };
            }
            return false;
        }
        let Some(event) = self.ext.handle_event(event, target, &self.window, &self.webview) else {
            return false;
        };

        let (window, state, callbacks) = (&self.window, &self.state, self.callbacks);
        #[allow(clippy::collapsible_match)]
        match event {
            Event::UserEvent(cmd) => {
                return dispatch_command(
                    cmd,
                    window,
                    &self.webview,
                    state,
                    callbacks,
                    &mut self.menu_items,
                    &mut self.current_menu,
                    &mut self.config,
                    &mut self.tray,
                );
            }
            Event::WindowEvent {
                event: ref win_event, ..
            } => {
                // These flags can change from the native window manager without a
                // Rustino command (for example, the user minimizes the window).
                state
                    .is_minimized
                    .store(window.is_minimized(), Ordering::Release);
                state
                    .is_fullscreen
                    .store(window.fullscreen().is_some(), Ordering::Release);

                match win_event {
                    WindowEvent::CloseRequested => {
                        if let Some(cb) = callbacks.on_closing {
                            if unsafe { cb(callbacks.context) } != 0 {
                                return false;
                            }
                        }
                        return true;
                    }
                    WindowEvent::Resized(size) => {
                        state.store_size(size.width, size.height);
                        state
                            .is_maximized
                            .store(window.is_maximized(), Ordering::Release);
                        if let Some(cb) = callbacks.on_resized {
                            unsafe {
                                cb(callbacks.context, size.width as i32, size.height as i32)
                            };
                        }
                    }
                    WindowEvent::Moved(pos) => {
                        state.store_position(pos.x, pos.y);
                        update_monitor_cache(window, state);
                        if let Some(cb) = callbacks.on_moved {
                            unsafe { cb(callbacks.context, pos.x, pos.y) };
                        }
                    }
                    WindowEvent::Focused(focused) => {
                        state.is_focused.store(*focused, Ordering::Release);
                        if let Some(cb) = callbacks.on_focus_changed {
                            unsafe {
                                cb(callbacks.context, if *focused { 1 } else { 0 })
                            };
                        }
                    }
                    WindowEvent::KeyboardInput { .. } => {
                        // Forward keyboard events to the webview by not consuming them
                        // The webview's internal handler will process these events
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        false
    }

    /// Tells the host that the window closed.
    pub(super) fn closed(&self) {
        if let Some(cb) = self.callbacks.on_closed {
            unsafe { cb(self.callbacks.context) };
        }
    }
}

/// Browser arguments of WebView2 for the settings it has no API for; `None` keeps wry's own.
/// They apply to this webview only (not through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, which
/// would reach every later window of the process). Windows sharing a user data folder need the
/// same arguments: WebView2 refuses to create the others.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn webview2_browser_args(config: &WindowConfig) -> Option<String> {
    let mut extra = Vec::new();
    if !config.web_security_enabled {
        extra.push("--disable-web-security");
    }
    if config.ignore_certificate_errors {
        extra.push("--ignore-certificate-errors");
    }
    if extra.is_empty() {
        return None;
    }
    // Custom arguments replace wry's defaults (wry 0.55): its mini menu and SmartScreen off,
    // autoplay without a user gesture
    let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if config.media_autoplay {
        args.push_str(" --autoplay-policy=no-user-gesture-required");
    }
    for arg in extra {
        args.push(' ');
        args.push_str(arg);
    }
    Some(args)
}

/// Settings that only WebView2 supports.
fn warn_unsupported_settings(#[allow(unused_variables)] config: &WindowConfig) {
    #[cfg(not(target_os = "windows"))]
    {
        if !config.web_security_enabled {
            config.logger().warning("[rustino] Warning: SetWebSecurityEnabled(false) is only supported on Windows (WebView2). Ignored on this platform.");
        }
        if config.ignore_certificate_errors {
            config.logger().warning("[rustino] Warning: SetIgnoreCertificateErrorsEnabled(true) is only supported on Windows (WebView2). Ignored on this platform.");
        }
    }
}

pub(super) fn center_window(window: &tao::window::Window) {
    if let Some(monitor) = window.current_monitor() {
        let monitor_size = monitor.size();
        let monitor_pos = monitor.position();
        let window_size = window.outer_size();
        let x = monitor_pos.x + ((monitor_size.width as i32 - window_size.width as i32) / 2);
        let y = monitor_pos.y + ((monitor_size.height as i32 - window_size.height as i32) / 2);
        window.set_outer_position(PhysicalPosition::new(x, y));
    }
}

pub(crate) fn handle_new_window_req(
    url: String,
    ctx: *mut std::ffi::c_void,
    cb: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char) -> i32,
) -> wry::NewWindowResponse {
    if let Ok(cstr) = CString::new(url) {
        unsafe { cb(ctx, cstr.as_ptr()) };
    }
    wry::NewWindowResponse::Deny
}

#[cfg(test)]
mod tests {
    use super::handle_new_window_req;
    use std::ffi::CStr;
    use std::sync::atomic::{AtomicBool, Ordering};

    static CALLBACK_CALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn mock_cb(_ctx: *mut std::ffi::c_void, url: *const std::ffi::c_char) -> i32 {
        let c_str = unsafe { CStr::from_ptr(url) };
        assert_eq!(c_str.to_str().unwrap(), "https://example.com");
        CALLBACK_CALLED.store(true, Ordering::SeqCst);
        0
    }

    #[test]
    fn test_handle_new_window_req() {
        CALLBACK_CALLED.store(false, Ordering::SeqCst);
        let resp = handle_new_window_req(
            "https://example.com".to_string(),
            std::ptr::null_mut(),
            mock_cb,
        );
        assert!(CALLBACK_CALLED.load(Ordering::SeqCst));
        match resp {
            wry::NewWindowResponse::Deny => {}
            _ => panic!("Expected Deny"),
        }
    }

    #[test]
    fn webview2_browser_args_keep_wry_defaults() {
        let mut config = crate::config::WindowConfig::default();
        assert_eq!(super::webview2_browser_args(&config), None, "wry's own arguments");
        config.web_security_enabled = false;
        config.ignore_certificate_errors = true;
        assert_eq!(
            super::webview2_browser_args(&config).unwrap(),
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
             --autoplay-policy=no-user-gesture-required --disable-web-security --ignore-certificate-errors"
        );
        config.media_autoplay = false;
        config.web_security_enabled = true;
        assert_eq!(
            super::webview2_browser_args(&config).unwrap(),
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --ignore-certificate-errors"
        );
    }
}
