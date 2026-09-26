//! Native window features beyond the basic window state: light/dark theme, taskbar progress,
//! attention requests, drag regions for chromeless windows, macOS title bar styles, less common
//! window flags and the macOS app events (opened URLs, Dock click).
//!
//! Their commands run in the event loop before `dispatch_command`, in
//! [`WindowExtRuntime::handle_event`], which also has the event loop target they need on macOS.

use std::ffi::{CString, c_char, c_void};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tao::event::{Event, WindowEvent};
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::window::{ProgressState, ResizeDirection, Theme, UserAttentionType, Window, WindowBuilder};
use wry::WebViewBuilder;

use crate::commands::RustinoCommand;
use crate::state::SharedState;

/// Messages of the drag region script, which the IPC handler keeps from the host.
const IPC_PREFIX: &str = "__rustino:";

/// Width of the edges that resize a chromeless window, in CSS pixels.
#[cfg(not(target_os = "macos"))]
const RESIZE_BORDER: u32 = 6;

/// Drags the window from the elements with `data-rustino-drag-region` (the element itself, not
/// its children, so buttons in a title bar stay clickable) and maximizes it on double click.
/// The drag starts when the mouse moves, so double clicks still reach the page. On chromeless
/// windows the edges of the page resize the window: the native side sets their width with
/// `setResizeBorder` (0 when the OS resizes the window or it can't be resized).
const DRAG_REGION_SCRIPT: &str = r#"(() => {
  if (window.__rustino_window) return;
  const post = (message) => window.ipc && window.ipc.postMessage('__rustino:' + message);
  const ATTRIBUTE = 'data-rustino-drag-region';
  const CURSORS = { n: 'ns-resize', s: 'ns-resize', e: 'ew-resize', w: 'ew-resize',
    ne: 'nesw-resize', sw: 'nesw-resize', nw: 'nwse-resize', se: 'nwse-resize' };
  let border = 0;
  let pressed = null;
  let cursorStyle = null;

  const edgeAt = (e) => {
    if (!border) return null;
    const v = e.clientY < border ? 'n' : e.clientY >= innerHeight - border ? 's' : '';
    const h = e.clientX < border ? 'w' : e.clientX >= innerWidth - border ? 'e' : '';
    return v + h || null;
  };
  const setCursor = (cursor) => {
    if (!cursor) {
      if (cursorStyle) cursorStyle.remove();
      return;
    }
    cursorStyle = cursorStyle || document.createElement('style');
    cursorStyle.textContent = '*{cursor:' + cursor + '!important}';
    if (!cursorStyle.isConnected) document.documentElement.appendChild(cursorStyle);
  };
  const isDragRegion = (el) =>
    el instanceof Element && el.hasAttribute(ATTRIBUTE) && el.getAttribute(ATTRIBUTE) !== 'false';

  addEventListener('mousemove', (e) => {
    if (pressed) {
      const dx = e.screenX - pressed.x, dy = e.screenY - pressed.y;
      if (!(e.buttons & 1)) pressed = null;
      else if (dx * dx + dy * dy > 4) { pressed = null; post('drag'); }
      return;
    }
    setCursor(CURSORS[edgeAt(e)]);
  }, true);
  addEventListener('mouseout', (e) => { if (!e.relatedTarget) setCursor(null); }, true);
  addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    const edge = edgeAt(e);
    if (edge) {
      e.preventDefault();
      e.stopImmediatePropagation();
      post('resize:' + edge);
    } else if (isDragRegion(e.target)) {
      e.preventDefault();
      if (e.detail === 2) { pressed = null; post('maximize'); }
      else pressed = { x: e.screenX, y: e.screenY };
    }
  }, true);
  addEventListener('mouseup', () => { pressed = null; }, true);

  Object.defineProperty(window, '__rustino_window', { value: Object.freeze({
    setResizeBorder: (width) => { border = width; if (!width) setCursor(null); },
  }) });
  post('ready');
})();"#;

/// Title bar styles on macOS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MacTitleBarStyle {
    #[default]
    Default,
    /// Transparent title bar over the window background, with the title.
    Transparent,
    /// The page extends under a transparent title bar without title, where only the traffic
    /// lights remain (the look of Slack or VS Code).
    Overlay,
}

impl MacTitleBarStyle {
    pub fn from_i32(style: i32) -> Self {
        match style {
            1 => Self::Transparent,
            2 => Self::Overlay,
            _ => Self::Default,
        }
    }
}

/// Options applied when the window is built.
pub struct WindowExtOptions {
    pub theme: Option<Theme>,
    /// `None` keeps the platform default (a shadow on Windows and macOS).
    pub shadow: Option<bool>,
    pub skip_taskbar: bool,
    pub content_protection: bool,
    pub visible_on_all_workspaces: bool,
    pub closable: bool,
    pub minimizable: bool,
    pub maximizable: bool,
    pub always_on_bottom: bool,
    pub ignore_cursor_events: bool,
    pub mac_title_bar_style: MacTitleBarStyle,
    pub traffic_light_position: Option<(f64, f64)>,
    pub desktop_file_name: Option<String>,
    /// Off for apps that load untrusted pages: every page can send the script's messages
    pub drag_regions: bool,
}

impl Default for WindowExtOptions {
    fn default() -> Self {
        Self {
            theme: None,
            shadow: None,
            skip_taskbar: false,
            content_protection: false,
            visible_on_all_workspaces: false,
            closable: true,
            minimizable: true,
            maximizable: true,
            always_on_bottom: false,
            ignore_cursor_events: false,
            mac_title_bar_style: MacTitleBarStyle::Default,
            traffic_light_position: None,
            desktop_file_name: None,
            drag_regions: true,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct WindowExtCallbacks {
    /// (context, theme): 1 light, 2 dark
    pub on_theme_changed: Option<unsafe extern "C" fn(*mut c_void, i32)>,
    /// (context, scale factor, new width, new height)
    pub on_scale_factor_changed: Option<unsafe extern "C" fn(*mut c_void, f64, i32, i32)>,
    /// (context, URLs separated by '\n'): macOS only
    pub on_urls_opened: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    /// (context, has visible windows): macOS only
    pub on_reopen: Option<unsafe extern "C" fn(*mut c_void, i32)>,
}

#[derive(Debug)]
pub enum WindowCommand {
    SetTheme(Option<Theme>),
    SetProgressBar(ProgressState, Option<u64>),
    RequestUserAttention(Option<UserAttentionType>),
    SetShadow(bool),
    SetSkipTaskbar(bool),
    SetContentProtection(bool),
    SetVisibleOnAllWorkspaces(bool),
    SetClosable(bool),
    SetMinimizable(bool),
    SetMaximizable(bool),
    SetAlwaysOnBottom(bool),
    SetIgnoreCursorEvents(bool),
    SetTrafficLightPosition(f64, f64),
    SetDesktopFileName(Option<String>),
    DragWindow,
    DragResizeWindow(ResizeDirection),
    /// Double click on a drag region
    ToggleMaximize,
    /// A page loaded the drag region script and waits for the resize border
    PageReady,
    /// The GTK theme settings changed (tao reports theme changes only on Windows and macOS)
    #[cfg(target_os = "linux")]
    CheckTheme,
}

/// Configuration of the window features until the window runs.
#[derive(Default)]
pub struct WindowExt {
    pub options: WindowExtOptions,
    pub callbacks: WindowExtCallbacks,
}

impl WindowExt {
    pub fn configure_window(&self, builder: WindowBuilder) -> WindowBuilder {
        let options = &self.options;
        #[allow(unused_mut)]
        let mut builder = builder
            .with_theme(options.theme)
            .with_closable(options.closable)
            .with_minimizable(options.minimizable)
            .with_maximizable(options.maximizable)
            .with_content_protection(options.content_protection)
            .with_visible_on_all_workspaces(options.visible_on_all_workspaces);
        // `with_always_on_bottom` also turns off always on top
        if options.always_on_bottom {
            builder = builder.with_always_on_bottom(true);
        }

        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowBuilderExtWindows;
            builder = builder.with_skip_taskbar(options.skip_taskbar);
            if let Some(shadow) = options.shadow {
                builder = builder.with_undecorated_shadow(shadow);
            }
        }
        #[cfg(target_os = "linux")]
        {
            use tao::platform::unix::WindowBuilderExtUnix;
            builder = builder.with_skip_taskbar(options.skip_taskbar);
        }
        #[cfg(target_os = "macos")]
        {
            use tao::platform::macos::WindowBuilderExtMacOS;
            if let Some(shadow) = options.shadow {
                builder = builder.with_has_shadow(shadow);
            }
            match options.mac_title_bar_style {
                MacTitleBarStyle::Default => {}
                MacTitleBarStyle::Transparent => builder = builder.with_titlebar_transparent(true),
                MacTitleBarStyle::Overlay => {
                    builder = builder
                        .with_titlebar_transparent(true)
                        .with_fullsize_content_view(true)
                        .with_title_hidden(true);
                }
            }
            if let Some((x, y)) = options.traffic_light_position {
                builder = builder.with_traffic_light_inset(tao::dpi::LogicalPosition::new(x, y));
            }
        }
        builder
    }

    pub fn configure_webview<'a>(&self, builder: WebViewBuilder<'a>) -> WebViewBuilder<'a> {
        // WebView2 has its own color scheme; WKWebView and WebKitGTK follow the app theme
        #[cfg(target_os = "windows")]
        let builder = {
            use wry::WebViewBuilderExtWindows;
            builder.with_theme(webview_theme(self.options.theme))
        };
        if self.options.drag_regions {
            builder.with_initialization_script(DRAG_REGION_SCRIPT)
        } else {
            builder
        }
    }

    pub fn ipc_filter(&self, proxy: EventLoopProxy<RustinoCommand>) -> IpcFilter {
        IpcFilter {
            proxy,
            enabled: self.options.drag_regions,
        }
    }

    pub fn start(
        self,
        window: &Window,
        state: &Arc<SharedState>,
        context: *mut c_void,
        #[allow(unused_variables)] proxy: EventLoopProxy<RustinoCommand>,
    ) -> WindowExtRuntime {
        state.store_theme(theme_code(window.theme()));
        state.store_scale_factor(window.scale_factor());
        if self.options.ignore_cursor_events {
            let _ = window.set_ignore_cursor_events(true);
        }
        #[cfg(target_os = "linux")]
        launcher::set_desktop_file_name(self.options.desktop_file_name.clone());
        WindowExtRuntime {
            callbacks: self.callbacks,
            context,
            state: Arc::clone(state),
            skip_taskbar: self.options.skip_taskbar,
            maximizable: self.options.maximizable,
            resize_border: None,
            resize_border_dirty: false,
            #[cfg(target_os = "linux")]
            _theme_watch: GtkThemeWatch::new(proxy),
        }
    }
}

/// Keeps the messages of the drag region script from the host and runs them in the event loop.
pub struct IpcFilter {
    proxy: EventLoopProxy<RustinoCommand>,
    /// Without drag regions all the messages go to the host
    enabled: bool,
}

impl IpcFilter {
    /// Returns false for the messages of the page.
    pub fn handle(&self, message: &str) -> bool {
        if !self.enabled {
            return false;
        }
        match parse_ipc_message(message) {
            Some(command) => {
                let _ = self.proxy.send_event(RustinoCommand::Window(command));
                true
            }
            None => false,
        }
    }
}

/// The window features while the window runs.
pub struct WindowExtRuntime {
    callbacks: WindowExtCallbacks,
    context: *mut c_void,
    state: Arc<SharedState>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    skip_taskbar: bool,
    /// tao's `is_maximizable` reads the zoom button on macOS, which chromeless windows lack
    maximizable: bool,
    /// Resize border last sent to the page
    resize_border: Option<u32>,
    /// The window state changed: check the resize border once the pending events are handled
    resize_border_dirty: bool,
    #[cfg(target_os = "linux")]
    _theme_watch: Option<GtkThemeWatch>,
}

impl WindowExtRuntime {
    /// Runs the window feature commands and follows the events the features depend on. Returns
    /// the events left for the rest of the event loop.
    pub fn handle_event<'a>(
        &mut self,
        event: Event<'a, RustinoCommand>,
        target: &EventLoopWindowTarget<RustinoCommand>,
        window: &Window,
        webview: &wry::WebView,
    ) -> Option<Event<'a, RustinoCommand>> {
        match event {
            Event::UserEvent(RustinoCommand::Window(command)) => {
                self.dispatch(command, target, window, webview);
                return None;
            }
            // The builder hides the taskbar button on Windows and Linux; on macOS the activation
            // policy of the app can change only after tao sets it, before `Init`
            #[cfg(target_os = "macos")]
            Event::NewEvents(tao::event::StartCause::Init) if self.skip_taskbar => {
                set_skip_taskbar(window, target, true);
            }
            Event::UserEvent(
                RustinoCommand::SetDecorations(_)
                | RustinoCommand::SetResizable(_)
                | RustinoCommand::SetMaximized(_)
                | RustinoCommand::SetMinimized(_)
                | RustinoCommand::SetFullscreen(_),
            )
            | Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => self.resize_border_dirty = true,
            Event::WindowEvent {
                event: WindowEvent::ThemeChanged(theme),
                ..
            } => self.theme_changed(theme),
            Event::WindowEvent {
                event:
                    WindowEvent::ScaleFactorChanged {
                        scale_factor,
                        ref new_inner_size,
                    },
                ..
            } => {
                self.state.store_scale_factor(scale_factor);
                if let Some(cb) = self.callbacks.on_scale_factor_changed {
                    let (width, height) = (new_inner_size.width as i32, new_inner_size.height as i32);
                    unsafe { cb(self.context, scale_factor, width, height) };
                }
            }
            Event::Opened { ref urls } => {
                if let Some(cb) = self.callbacks.on_urls_opened {
                    let urls: Vec<&str> = urls.iter().map(|url| url.as_str()).collect();
                    if let Ok(cstr) = CString::new(urls.join("\n")) {
                        unsafe { cb(self.context, cstr.as_ptr()) };
                    }
                }
            }
            Event::Reopen {
                has_visible_windows,
                ..
            } => {
                if let Some(cb) = self.callbacks.on_reopen {
                    unsafe { cb(self.context, i32::from(has_visible_windows)) };
                }
            }
            Event::MainEventsCleared if self.resize_border_dirty => {
                self.resize_border_dirty = false;
                self.update_resize_border(window, webview, false);
            }
            _ => {}
        }
        Some(event)
    }

    fn dispatch(
        &mut self,
        command: WindowCommand,
        target: &EventLoopWindowTarget<RustinoCommand>,
        window: &Window,
        #[allow(unused_variables)] webview: &wry::WebView,
    ) {
        match command {
            WindowCommand::SetTheme(theme) => {
                window.set_theme(theme);
                #[cfg(target_os = "windows")]
                {
                    use wry::WebViewExtWindows;
                    let _ = webview.set_theme(webview_theme(theme));
                }
                // macOS and Linux don't report the themes set by the app
                self.theme_changed(window.theme());
            }
            WindowCommand::SetProgressBar(state, progress) => set_progress_bar(window, state, progress),
            WindowCommand::RequestUserAttention(kind) => window.request_user_attention(kind),
            #[allow(unused_variables)]
            WindowCommand::SetShadow(shadow) => {
                #[cfg(target_os = "windows")]
                {
                    use tao::platform::windows::WindowExtWindows;
                    window.set_undecorated_shadow(shadow);
                }
                #[cfg(target_os = "macos")]
                {
                    use tao::platform::macos::WindowExtMacOS;
                    window.set_has_shadow(shadow);
                }
            }
            WindowCommand::SetSkipTaskbar(skip) => set_skip_taskbar(window, target, skip),
            WindowCommand::SetContentProtection(enabled) => window.set_content_protection(enabled),
            WindowCommand::SetVisibleOnAllWorkspaces(visible) => {
                window.set_visible_on_all_workspaces(visible)
            }
            WindowCommand::SetClosable(closable) => window.set_closable(closable),
            WindowCommand::SetMinimizable(minimizable) => window.set_minimizable(minimizable),
            WindowCommand::SetMaximizable(maximizable) => {
                window.set_maximizable(maximizable);
                self.maximizable = maximizable;
            }
            WindowCommand::SetAlwaysOnBottom(on_bottom) => window.set_always_on_bottom(on_bottom),
            WindowCommand::SetIgnoreCursorEvents(ignore) => {
                let _ = window.set_ignore_cursor_events(ignore);
            }
            #[allow(unused_variables)]
            WindowCommand::SetTrafficLightPosition(x, y) => {
                #[cfg(target_os = "macos")]
                {
                    use tao::platform::macos::WindowExtMacOS;
                    window.set_traffic_light_inset(tao::dpi::LogicalPosition::new(x, y));
                }
            }
            #[allow(unused_variables)]
            WindowCommand::SetDesktopFileName(name) => {
                #[cfg(target_os = "linux")]
                launcher::set_desktop_file_name(name);
            }
            WindowCommand::DragWindow => {
                let _ = window.drag_window();
            }
            WindowCommand::DragResizeWindow(direction) => {
                let _ = window.drag_resize_window(direction);
            }
            WindowCommand::ToggleMaximize => {
                if window.is_resizable() && self.maximizable {
                    let maximized = !window.is_maximized();
                    window.set_maximized(maximized);
                    self.state.is_maximized.store(maximized, Ordering::Release);
                    self.resize_border_dirty = true;
                }
            }
            WindowCommand::PageReady => self.update_resize_border(window, webview, true),
            #[cfg(target_os = "linux")]
            WindowCommand::CheckTheme => self.theme_changed(window.theme()),
        }
    }

    fn theme_changed(&self, theme: Theme) {
        let code = theme_code(theme);
        if self.state.store_theme(code)
            && let Some(cb) = self.callbacks.on_theme_changed
        {
            unsafe { cb(self.context, i32::from(code)) };
        }
    }

    fn update_resize_border(&mut self, window: &Window, webview: &wry::WebView, force: bool) {
        let border = resize_border(window);
        if force || self.resize_border != Some(border) {
            self.resize_border = Some(border);
            let _ = webview.evaluate_script(&format!(
                "window.__rustino_window && window.__rustino_window.setResizeBorder({border})"
            ));
        }
    }
}

/// Width of the edges that resize the window from the page: the web content covers the resize
/// borders of chromeless windows on Windows and Linux. macOS keeps them.
fn resize_border(#[allow(unused_variables)] window: &Window) -> u32 {
    #[cfg(target_os = "macos")]
    {
        0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let resizable = !window.is_decorated()
            && window.is_resizable()
            && !window.is_maximized()
            && window.fullscreen().is_none();
        if resizable { RESIZE_BORDER } else { 0 }
    }
}

#[allow(unused_variables)]
fn set_skip_taskbar(window: &Window, target: &EventLoopWindowTarget<RustinoCommand>, skip: bool) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        let _ = window.set_skip_taskbar(skip);
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = window.set_skip_taskbar(skip);
    }
    // macOS has no taskbar button per window: the app leaves the Dock and the app switcher
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopWindowTargetExtMacOS};
        let policy = if skip { ActivationPolicy::Accessory } else { ActivationPolicy::Regular };
        target.set_activation_policy_at_runtime(policy);
    }
}

fn set_progress_bar(window: &Window, state: ProgressState, progress: Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        let _ = window;
        launcher::set_progress(state, progress);
    }
    #[cfg(not(target_os = "linux"))]
    window.set_progress_bar(tao::window::ProgressBarState {
        state: Some(state),
        progress,
        desktop_filename: None,
    });
}

#[cfg(target_os = "windows")]
fn webview_theme(theme: Option<Theme>) -> wry::Theme {
    match theme {
        Some(Theme::Dark) => wry::Theme::Dark,
        Some(Theme::Light) => wry::Theme::Light,
        _ => wry::Theme::Auto,
    }
}

/// 1 light, 2 dark: the values of the host API
fn theme_code(theme: Theme) -> u8 {
    if theme == Theme::Dark { 2 } else { 1 }
}

/// 0 follows the system
pub fn theme_from_i32(theme: i32) -> Option<Theme> {
    match theme {
        1 => Some(Theme::Light),
        2 => Some(Theme::Dark),
        _ => None,
    }
}

pub fn progress_state_from_i32(state: i32) -> ProgressState {
    match state {
        1 => ProgressState::Normal,
        2 => ProgressState::Indeterminate,
        3 => ProgressState::Paused,
        4 => ProgressState::Error,
        _ => ProgressState::None,
    }
}

/// 0 cancels the request
pub fn attention_from_i32(kind: i32) -> Option<UserAttentionType> {
    match kind {
        1 => Some(UserAttentionType::Informational),
        2 => Some(UserAttentionType::Critical),
        _ => None,
    }
}

/// Clockwise from north
pub fn direction_from_i32(direction: i32) -> Option<ResizeDirection> {
    Some(match direction {
        0 => ResizeDirection::North,
        1 => ResizeDirection::NorthEast,
        2 => ResizeDirection::East,
        3 => ResizeDirection::SouthEast,
        4 => ResizeDirection::South,
        5 => ResizeDirection::SouthWest,
        6 => ResizeDirection::West,
        7 => ResizeDirection::NorthWest,
        _ => return None,
    })
}

fn parse_ipc_message(message: &str) -> Option<WindowCommand> {
    Some(match message.strip_prefix(IPC_PREFIX)? {
        "drag" => WindowCommand::DragWindow,
        "maximize" => WindowCommand::ToggleMaximize,
        "ready" => WindowCommand::PageReady,
        other => {
            let direction = match other.strip_prefix("resize:")? {
                "n" => ResizeDirection::North,
                "ne" => ResizeDirection::NorthEast,
                "e" => ResizeDirection::East,
                "se" => ResizeDirection::SouthEast,
                "s" => ResizeDirection::South,
                "sw" => ResizeDirection::SouthWest,
                "w" => ResizeDirection::West,
                "nw" => ResizeDirection::NorthWest,
                _ => return None,
            };
            WindowCommand::DragResizeWindow(direction)
        }
    })
}

/// Theme listeners on the GTK settings, which the windows of a screen share: they go away with
/// the window.
#[cfg(target_os = "linux")]
struct GtkThemeWatch {
    settings: gtk::Settings,
    handlers: Vec<gtk::glib::SignalHandlerId>,
}

#[cfg(target_os = "linux")]
impl GtkThemeWatch {
    fn new(proxy: EventLoopProxy<RustinoCommand>) -> Option<Self> {
        use gtk::prelude::ObjectExt;
        let settings = gtk::Settings::default()?;
        let handlers = ["gtk-theme-name", "gtk-application-prefer-dark-theme"]
            .into_iter()
            .map(|property| {
                let proxy = proxy.clone();
                settings.connect_notify_local(Some(property), move |_, _| {
                    let _ = proxy.send_event(RustinoCommand::Window(WindowCommand::CheckTheme));
                })
            })
            .collect();
        Some(Self { settings, handlers })
    }
}

#[cfg(target_os = "linux")]
impl Drop for GtkThemeWatch {
    fn drop(&mut self) {
        use gtk::prelude::ObjectExt;
        for handler in self.handlers.drain(..) {
            self.settings.disconnect(handler);
        }
    }
}

/// Badge and progress on the dock icon through the Unity LauncherEntry D-Bus signals, which
/// Ubuntu Dock, Dash to Dock and KDE Plasma follow. tao sends them through libunity only while
/// Unity itself runs.
#[cfg(target_os = "linux")]
pub mod launcher {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use gtk::gio;
    use gtk::glib::{ToVariant, Variant};
    use tao::window::ProgressState;

    /// The dock entry is the app's, shared by its windows
    static DESKTOP_FILE_NAME: Mutex<Option<String>> = Mutex::new(None);

    pub fn set_desktop_file_name(name: Option<String>) {
        if let Ok(mut current) = DESKTOP_FILE_NAME.lock() {
            *current = name;
        }
    }

    pub fn set_count(count: Option<u32>) {
        let count = count.unwrap_or(0);
        update(&[
            ("count", i64::from(count).to_variant()),
            ("count-visible", (count > 0).to_variant()),
        ]);
    }

    pub fn set_progress(state: ProgressState, progress: Option<u64>) {
        let visible = !matches!(state, ProgressState::None);
        let mut properties = vec![("progress-visible", visible.to_variant())];
        if let Some(progress) = progress {
            properties.push(("progress", (progress.min(100) as f64 / 100.0).to_variant()));
        }
        update(&properties);
    }

    fn update(properties: &[(&str, Variant)]) {
        let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
            return;
        };
        let app_uri = app_uri();
        let properties: HashMap<String, Variant> =
            properties.iter().map(|(key, value)| (key.to_string(), value.clone())).collect();
        let path = format!("/com/canonical/unity/launcherentry/{}", path_id(&app_uri));
        let _ = connection.emit_signal(
            None,
            &path,
            "com.canonical.Unity.LauncherEntry",
            "Update",
            Some(&(app_uri, properties).to_variant()),
        );
    }

    /// `application://<desktop file>`, by default `<executable name>.desktop`
    pub(super) fn app_uri() -> String {
        let name = DESKTOP_FILE_NAME.lock().ok().and_then(|name| name.clone()).unwrap_or_else(|| {
            let exe = std::env::current_exe().ok();
            let stem = exe.as_deref().and_then(|p| p.file_stem()).and_then(|s| s.to_str());
            format!("{}.desktop", stem.unwrap_or("rustino"))
        });
        format!("application://{name}")
    }

    /// Any object path works: the docks read the app from the signal
    pub(super) fn path_id(app_uri: &str) -> u32 {
        app_uri.bytes().fold(5381u32, |hash, b| hash.wrapping_mul(33).wrapping_add(u32::from(b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_messages_of_the_script() {
        assert!(matches!(parse_ipc_message("__rustino:drag"), Some(WindowCommand::DragWindow)));
        assert!(matches!(parse_ipc_message("__rustino:maximize"), Some(WindowCommand::ToggleMaximize)));
        assert!(matches!(parse_ipc_message("__rustino:ready"), Some(WindowCommand::PageReady)));
        assert!(matches!(
            parse_ipc_message("__rustino:resize:nw"),
            Some(WindowCommand::DragResizeWindow(ResizeDirection::NorthWest))
        ));
        assert!(matches!(
            parse_ipc_message("__rustino:resize:e"),
            Some(WindowCommand::DragResizeWindow(ResizeDirection::East))
        ));
    }

    #[test]
    fn page_messages_reach_the_host() {
        assert!(parse_ipc_message("drag").is_none());
        assert!(parse_ipc_message("__rustino:resize:x").is_none());
        assert!(parse_ipc_message("__rustino:unknown").is_none());
        assert!(parse_ipc_message("{\"type\":\"__rustino:drag\"}").is_none());
    }

    #[test]
    fn host_values() {
        assert_eq!(theme_from_i32(0), None);
        assert_eq!(theme_from_i32(1), Some(Theme::Light));
        assert_eq!(theme_from_i32(2), Some(Theme::Dark));
        assert_eq!(theme_code(Theme::Light), 1);
        assert_eq!(theme_code(Theme::Dark), 2);
        assert!(matches!(progress_state_from_i32(0), ProgressState::None));
        assert!(matches!(progress_state_from_i32(3), ProgressState::Paused));
        assert!(matches!(progress_state_from_i32(99), ProgressState::None));
        assert!(attention_from_i32(0).is_none());
        assert!(matches!(attention_from_i32(2), Some(UserAttentionType::Critical)));
        assert!(matches!(direction_from_i32(0), Some(ResizeDirection::North)));
        assert!(matches!(direction_from_i32(7), Some(ResizeDirection::NorthWest)));
        assert!(direction_from_i32(8).is_none());
        assert_eq!(MacTitleBarStyle::from_i32(2), MacTitleBarStyle::Overlay);
        assert_eq!(MacTitleBarStyle::from_i32(-1), MacTitleBarStyle::Default);
    }

    #[test]
    fn default_options_keep_the_window_as_is() {
        let options = WindowExtOptions::default();
        assert!(options.theme.is_none());
        assert!(options.shadow.is_none());
        assert!(options.closable && options.minimizable && options.maximizable);
        assert!(options.drag_regions);
        assert!(!options.skip_taskbar && !options.content_protection && !options.ignore_cursor_events);
        assert_eq!(options.mac_title_bar_style, MacTitleBarStyle::Default);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn launcher_entry_uri() {
        launcher::set_desktop_file_name(Some("com.example.App.desktop".into()));
        assert_eq!(launcher::app_uri(), "application://com.example.App.desktop");
        assert_eq!(launcher::path_id("a"), 5381 * 33 + 97);
        launcher::set_desktop_file_name(None);
        assert!(launcher::app_uri().ends_with(".desktop"));
    }
}
