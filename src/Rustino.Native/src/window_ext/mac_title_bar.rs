//! The title bar of macOS: its styles, and the traffic lights under the overlay title bar.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use tao::event::{Event, WindowEvent};
use tao::window::Window;

use crate::commands::RustinoCommand;
use crate::window::CommandProxy;

use super::{MacTitleBarStyle, WindowCommand, WindowControls};

/// The title bar style while the window runs.
pub(super) struct MacTitleBar {
    style: MacTitleBarStyle,
    /// Where the host placed the traffic lights
    pub(super) traffic_lights: Option<(f64, f64)>,
    /// The decorations are coming back: tao rebuilds the style mask asynchronously, without the
    /// style, which goes back on the resize that follows
    pending: bool,
    /// Where the traffic lights sit before Rustino moves them: they go back there when the
    /// overlay title bar loses its height
    default_lights: Option<TrafficLights>,
    /// Where Rustino put the traffic lights: tao's inset and the left and top margins it gives
    placed: Option<((f64, f64), (f64, f64))>,
    /// Follows what the event loop sees too late: animated resizes and the way out of fullscreen
    observer: Option<Retained<WindowObserver>>,
    proxy: CommandProxy,
    /// Checks in a row that found the lights moved back
    checks: u8,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class doesn't implement `Drop`
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RustinoTitleBarObserver"]
    #[ivars = Observed]
    struct WindowObserver;

    impl WindowObserver {
        // AppKit puts the lights back at each step of a resize. The event loop moves them again
        // on the `Resized` event, but it doesn't run during an animated resize (zoom, double
        // click on the title bar): the lights would stay in their default place until its end.
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &AnyObject) {
            let Some((x, y)) = self.ivars().inset.get() else {
                return;
            };
            let ns_window: *mut AnyObject = unsafe { msg_send![notification, object] };
            if !ns_window.is_null() && !has_fullscreen_mask(ns_window) {
                let state = self.ivars();
                state.placing.set(true);
                unsafe {
                    layout_title_bar(ns_window);
                    move_traffic_lights(ns_window, x, y);
                }
                state.placing.set(false);
            }
        }

        // AppKit lays out the title bar again, with the lights back in their default place, on
        // changes it doesn't tell about (the theme, the style, the decorations, ...): the
        // event loop checks the lights as soon as it has laid them out.
        #[unsafe(method(titleBarFrameDidChange:))]
        fn title_bar_frame_did_change(&self, _notification: &AnyObject) {
            let state = self.ivars();
            if state.watching.get()
                && !state.placing.get()
                && !state.check_requested.get()
                && state.inset.get().is_some()
            {
                state.check_requested.set(true);
                let _ = state.proxy.send_event(RustinoCommand::Window(WindowCommand::CheckTrafficLights));
            }
        }

        // The animation that leaves fullscreen shows the lights, back in the window, over the
        // page as it is when the animation starts. tao reports the window as windowed only at
        // its end, when the user leaves fullscreen: until then the page would have no room for
        // the lights, which would sit over it in their default place.
        #[unsafe(method(windowWillExitFullScreen:))]
        fn window_will_exit_full_screen(&self, _notification: &AnyObject) {
            let state = self.ivars();
            state.leaving_fullscreen.set(Some(Instant::now()));
            // Now, for the page to make room before the animation starts
            if let Some((webview, script)) = &*state.windowed_chrome.borrow() {
                let script = objc2_foundation::NSString::from_str(script);
                let _: () = unsafe {
                    msg_send![
                        &**webview,
                        evaluateJavaScript: &*script,
                        completionHandler: std::ptr::null_mut::<AnyObject>(),
                    ]
                };
                state.chrome_pushed.set(true);
            }
            // The lights come back to the window after this notification: the event loop
            // places them then
            let _ = state.proxy.send_event(RustinoCommand::Window(WindowCommand::CheckTrafficLights));
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn window_did_exit_full_screen(&self, _notification: &AnyObject) {
            let state = self.ivars();
            state.leaving_fullscreen.set(None);
            let _ = state.proxy.send_event(RustinoCommand::Window(WindowCommand::CheckTrafficLights));
        }
    }
);

/// What the observer of a window shares with its `MacTitleBar`.
struct Observed {
    proxy: CommandProxy,
    /// tao's inset of the lights, while Rustino places them
    inset: Cell<Option<(f64, f64)>>,
    /// When the window started to leave fullscreen, until it's windowed again
    leaving_fullscreen: Cell<Option<Instant>>,
    /// The webview and the script that gives its page the window controls, as last run while
    /// the window was not in fullscreen
    windowed_chrome: RefCell<Option<(Retained<AnyObject>, String)>>,
    /// The observer ran that script: the page no longer has the chrome the event loop gave it
    chrome_pushed: Cell<bool>,
    /// The view AppKit lays out the title bar in, observed for the changes of its frame
    title_bar: RefCell<Option<Retained<AnyObject>>>,
    /// AppKit's changes to the title bar ask for a check of the lights. Not when it keeps
    /// undoing what Rustino does: the two would move the lights in turn, without end
    watching: Cell<bool>,
    /// Rustino is placing the lights: the changes to the title bar are its own
    placing: Cell<bool>,
    /// A check of the lights is on its way to the event loop
    check_requested: Cell<bool>,
}

impl WindowObserver {
    /// Observes the window until dropped by `MacTitleBar`.
    fn new(ns_window: *mut AnyObject, proxy: CommandProxy, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Observed {
            proxy,
            inset: Cell::new(None),
            leaving_fullscreen: Cell::new(None),
            windowed_chrome: RefCell::new(None),
            chrome_pushed: Cell::new(false),
            title_bar: RefCell::new(None),
            watching: Cell::new(true),
            placing: Cell::new(false),
            check_requested: Cell::new(false),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let observe = |name: &str, selector: objc2::runtime::Sel| unsafe {
            let center: *mut AnyObject = msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
            let name = objc2_foundation::NSString::from_str(name);
            let _: () = msg_send![center, addObserver: &*this, selector: selector, name: &*name, object: ns_window];
        };
        observe("NSWindowDidResizeNotification", sel!(windowDidResize:));
        observe("NSWindowWillExitFullScreenNotification", sel!(windowWillExitFullScreen:));
        observe("NSWindowDidExitFullScreenNotification", sel!(windowDidExitFullScreen:));
        this
    }

    /// Observes the view that holds the lights of the window, which changes with its decorations.
    fn watch_title_bar(&self, ns_window: *mut AnyObject) {
        let name = objc2_foundation::NSString::from_str("NSViewFrameDidChangeNotification");
        unsafe {
            let close: *mut AnyObject = msg_send![ns_window, standardWindowButton: 0usize];
            if close.is_null() {
                return;
            }
            let title_bar: *mut AnyObject = msg_send![close, superview];
            let container: *mut AnyObject = msg_send![title_bar, superview];
            let mut watched = self.ivars().title_bar.borrow_mut();
            if container.is_null() || watched.as_ref().is_some_and(|view| Retained::as_ptr(view) == container.cast_const()) {
                return;
            }
            let center: *mut AnyObject = msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
            if let Some(old) = watched.take() {
                let _: () = msg_send![center, removeObserver: self, name: &*name, object: &*old];
            }
            let _: () = msg_send![container, setPostsFrameChangedNotifications: true];
            let _: () = msg_send![
                center,
                addObserver: self,
                selector: sel!(titleBarFrameDidChange:),
                name: &*name,
                object: container,
            ];
            *watched = Retained::retain(container);
        }
    }

    /// A transition that never ends (AppKit can fail to leave fullscreen) is over after a while.
    fn leaving_fullscreen(&self) -> bool {
        self.ivars().leaving_fullscreen.get().is_some_and(|since| since.elapsed() < LEAVING_FULLSCREEN_TIMEOUT)
    }
}

/// Longer than the animation that leaves fullscreen (about half a second).
const LEAVING_FULLSCREEN_TIMEOUT: Duration = Duration::from_secs(3);

impl Drop for MacTitleBar {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.take() {
            unsafe {
                let center: *mut AnyObject = msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
                let _: () = msg_send![center, removeObserver: &*observer];
            }
        }
    }
}

/// The traffic lights in the window, in points from its top left corner.
#[derive(Debug, Clone, Copy)]
pub(super) struct TrafficLights {
    /// Left edge of the close button
    left: f64,
    /// Right edge of the zoom button
    right: f64,
    top: f64,
    height: f64,
}

impl MacTitleBar {
    pub(super) fn new(
        style: MacTitleBarStyle,
        traffic_lights: Option<(f64, f64)>,
        window: &Window,
        proxy: CommandProxy,
    ) -> Self {
        use tao::platform::macos::WindowExtMacOS;
        // The event loop runs on the main thread
        let observer = MainThreadMarker::new()
            .map(|mtm| WindowObserver::new(window.ns_window().cast(), proxy.clone(), mtm));
        Self {
            style,
            traffic_lights,
            pending: false,
            default_lights: None,
            placed: None,
            observer,
            proxy,
            checks: 0,
        }
    }

    /// Records where the lights are, also for the resizes that follow.
    fn set_placed(&mut self, placed: Option<((f64, f64), (f64, f64))>) {
        self.placed = placed;
        if let Some(observer) = &self.observer {
            observer.ivars().inset.set(placed.map(|(inset, _)| inset));
        }
    }

    /// In fullscreen the window has no lights. tao and the style mask tell, each ahead of the
    /// other in some transitions; on the way out only the style mask does: the lights are back
    /// as soon as it changes.
    fn in_fullscreen(&self, window: &Window) -> bool {
        use tao::platform::macos::WindowExtMacOS;
        let mask = has_fullscreen_mask(window.ns_window().cast());
        if self.observer.as_ref().is_some_and(|observer| observer.leaving_fullscreen()) {
            mask
        } else {
            mask || window.fullscreen().is_some()
        }
    }

    /// The window started to leave fullscreen and its page already has the window controls,
    /// but the lights aren't back yet: the chrome waits for them.
    pub(super) fn leaving_fullscreen(&self, window: &Window) -> bool {
        self.observer.as_ref().is_some_and(|observer| observer.leaving_fullscreen()) && self.in_fullscreen(window)
    }

    /// Whether the observer gave the page the window controls since the last call: the chrome
    /// the event loop sent before is no longer the one of the page.
    pub(super) fn take_chrome_pushed(&self) -> bool {
        self.observer.as_ref().is_some_and(|observer| observer.ivars().chrome_pushed.replace(false))
    }

    /// The event loop got the check that the observer asked for.
    pub(super) fn check_received(&self) {
        if let Some(observer) = &self.observer {
            observer.ivars().check_requested.set(false);
        }
    }

    /// Keeps the script that gives the page the window controls (`None` without controls), to
    /// run it again as soon as the window leaves fullscreen.
    pub(super) fn remember_chrome(&self, window: &Window, webview: &wry::WebView, script: Option<&str>) {
        use wry::WebViewExtMacOS;
        let Some(observer) = &self.observer else { return };
        if self.in_fullscreen(window) {
            return;
        }
        let chrome = script.and_then(|script| {
            let webview = Retained::as_ptr(&webview.webview()) as *mut AnyObject;
            Some((unsafe { Retained::retain(webview) }?, script.to_owned()))
        });
        *observer.ivars().windowed_chrome.borrow_mut() = chrome;
    }

    /// Called before `dispatch_command` runs the event.
    pub(super) fn follow(&mut self, event: &Event<RustinoCommand>, window: &Window) {
        match event {
            Event::UserEvent(RustinoCommand::SetDecorations(true)) if !window.is_decorated() => {
                self.pending = true;
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } if self.pending => {
                self.pending = false;
                self.apply(window);
            }
            _ => {}
        }
    }

    pub(super) fn set_style(&mut self, style: MacTitleBarStyle, window: &Window) {
        self.style = style;
        // In fullscreen the page gets the new chrome only when the window leaves it: not the
        // one of the old style first
        if let Some(observer) = &self.observer {
            observer.ivars().windowed_chrome.borrow_mut().take();
        }
        // Chromeless windows get the style with their decorations
        if window.is_decorated() && !self.pending {
            self.apply(window);
        }
    }

    fn apply(&self, window: &Window) {
        use tao::platform::macos::WindowExtMacOS;
        let overlay = self.style == MacTitleBarStyle::Overlay;
        window.set_titlebar_transparent(self.style != MacTitleBarStyle::Default);
        window.set_fullsize_content_view(overlay);
        // tao hides the title only in the builder. 0 visible, 1 hidden (NSWindowTitleVisibility)
        let ns_window = window.ns_window() as *mut AnyObject;
        let visibility: isize = if overlay { 1 } else { 0 };
        unsafe {
            let _: () = msg_send![ns_window, setTitleVisibility: visibility];
        }
    }

    /// The traffic lights the page goes under, with the overlay title bar: the page leaves them
    /// their width and, unless the host sets a height, as high a title bar as their margin above
    /// gives them.
    pub(super) fn controls(&mut self, window: &Window, height: Option<u32>) -> Option<WindowControls> {
        if self.pending || self.in_fullscreen(window) {
            return None;
        }
        let overlay = self.style == MacTitleBarStyle::Overlay && window.is_decorated();
        self.place_lights(window, height.filter(|_| overlay));
        if !overlay {
            return None;
        }
        let lights = traffic_lights(window)?;
        let height = height.unwrap_or((2.0 * lights.top + lights.height).round() as u32);
        Some(WindowControls {
            platform: "macos",
            maximized: false,
            color: None,
            height: Some(height),
            left: Vec::new(),
            right: Vec::new(),
            native_left: (lights.right + lights.left).round() as u32,
            scale_factor: window.scale_factor(),
            labels: None,
        })
    }

    /// Moves the traffic lights where the host placed them or, by default, centers them in the
    /// title bar height set by the host, as far from the left edge as from the top. Without a
    /// height they go back where they were.
    fn place_lights(&mut self, window: &Window, height: Option<u32>) {
        use tao::platform::macos::WindowExtMacOS;
        if let Some(observer) = &self.observer {
            observer.ivars().placing.set(true);
        }
        self.place_lights_now(window, height);
        if let Some(observer) = &self.observer {
            let state = observer.ivars();
            state.placing.set(false);
            // AppKit moved the lights back after each of the last checks: no more of them
            // until the lights stay in place
            state.watching.set(self.checks < MAX_CHECKS);
            observer.watch_title_bar(window.ns_window().cast());
        }
    }

    fn place_lights_now(&mut self, window: &Window, height: Option<u32>) {
        use tao::platform::macos::WindowExtMacOS;
        unsafe { layout_title_bar(window.ns_window().cast()) };
        let Some(lights) = traffic_lights(window) else { return };
        if self.placed.is_none() && self.traffic_lights.is_none() {
            self.default_lights = Some(lights);
        }
        let at = |(left, top): (f64, f64)| (lights.left - left).abs() < 0.5 && (lights.top - top).abs() < 0.5;
        if let Some((x, y)) = self.traffic_lights {
            if self.placed.is_some_and(|(inset, margins)| inset == (x, y) && at(margins)) {
                self.checks = 0;
                return;
            }
            set_traffic_light_inset_now(window, x, y);
            let placed = traffic_lights(window).map(|lights| ((x, y), (lights.left, lights.top)));
            self.set_placed(placed);
        } else {
            let target = match (height, self.default_lights) {
                (Some(height), Some(default)) => {
                    let margin = ((f64::from(height) - default.height) / 2.0).max(0.0);
                    (margin, margin)
                }
                (None, Some(default)) if self.placed.is_some() => (default.left, default.top),
                _ => return,
            };
            if at(target) {
                if height.is_none() {
                    self.set_placed(None);
                }
                self.checks = 0;
                return;
            }
            // tao's y isn't the top margin of the lights: measure and correct
            let (x, top) = target;
            let mut y = self.placed.map_or(top, |((_, y), (_, old_top))| y - old_top + top);
            for _ in 0..2 {
                y = y.clamp(0.0, MAX_TRAFFIC_LIGHT_INSET);
                set_traffic_light_inset_now(window, x, y);
                match traffic_lights(window) {
                    Some(lights) if (lights.top - top).abs() > 0.25 => y += top - lights.top,
                    _ => break,
                }
            }
            self.set_placed(height.map(|_| ((x, y), target)));
        }
        // AppKit lays out the title bar again after a change of style or decorations, later
        // and without telling: check once it's done, a few times at most
        if self.checks < MAX_CHECKS {
            self.checks += 1;
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let _ = proxy.send_event(RustinoCommand::Window(WindowCommand::CheckTrafficLights));
            });
        }
    }
}

/// Checks in a row that can find the lights moved back by AppKit, before giving up.
const MAX_CHECKS: u8 = 3;

/// Bounds tao's y for the traffic lights, against measures taken while AppKit moves them.
const MAX_TRAFFIC_LIGHT_INSET: f64 = 200.0;

fn has_fullscreen_mask(ns_window: *mut AnyObject) -> bool {
    // NSWindowStyleMaskFullScreen
    const FULL_SCREEN: usize = 1 << 14;
    let mask: usize = unsafe { msg_send![ns_window, styleMask] };
    mask & FULL_SCREEN != 0
}

/// AppKit puts the lights back in their place in a layout pass that runs only after a resize or
/// the return of the decorations: runs it now, so that what moves the lights afterwards stays.
unsafe fn layout_title_bar(ns_window: *mut AnyObject) {
    unsafe {
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        let frame_view: *mut AnyObject = msg_send![content, superview];
        if !frame_view.is_null() {
            let _: () = msg_send![frame_view, layoutSubtreeIfNeeded];
        }
    }
}

/// tao stores the inset for its `drawRect:`, which the webview over the whole window keeps from
/// running, and for leaving fullscreen: the lights move now as tao would move them.
fn set_traffic_light_inset_now(window: &Window, x: f64, y: f64) {
    use tao::platform::macos::WindowExtMacOS;
    window.set_traffic_light_inset(tao::dpi::LogicalPosition::new(x, y));
    unsafe { move_traffic_lights(window.ns_window().cast(), x, y) };
}

/// Moves the lights as tao's `inset_traffic_lights` does.
unsafe fn move_traffic_lights(ns_window: *mut AnyObject, x: f64, y: f64) {
    use objc2_foundation::{NSPoint, NSRect};
    unsafe {
        // NSWindowCloseButton, NSWindowMiniaturizeButton and NSWindowZoomButton
        let buttons: [*mut AnyObject; 3] = [
            msg_send![ns_window, standardWindowButton: 0usize],
            msg_send![ns_window, standardWindowButton: 1usize],
            msg_send![ns_window, standardWindowButton: 2usize],
        ];
        if buttons.iter().any(|button| button.is_null()) {
            return;
        }
        let close: NSRect = msg_send![buttons[0], frame];
        let miniaturize: NSRect = msg_send![buttons[1], frame];
        let titlebar: *mut AnyObject = msg_send![buttons[0], superview];
        let container: *mut AnyObject = msg_send![titlebar, superview];
        if container.is_null() {
            return;
        }
        let window_frame: NSRect = msg_send![ns_window, frame];
        let mut container_frame: NSRect = msg_send![container, frame];
        container_frame.size.height = close.size.height + y;
        container_frame.origin.y = window_frame.size.height - container_frame.size.height;
        let _: () = msg_send![container, setFrame: container_frame];
        let spacing = miniaturize.origin.x - close.origin.x;
        for (i, &button) in buttons.iter().enumerate() {
            let frame: NSRect = msg_send![button, frame];
            let origin = NSPoint::new(x + i as f64 * spacing, frame.origin.y);
            let _: () = msg_send![button, setFrameOrigin: origin];
        }
    }
}

/// `None` when the window has no traffic lights (chromeless).
fn traffic_lights(window: &Window) -> Option<TrafficLights> {
    use objc2_foundation::NSRect;
    use tao::platform::macos::WindowExtMacOS;
    let ns_window = window.ns_window() as *mut AnyObject;
    unsafe {
        // NSWindowCloseButton and NSWindowZoomButton
        let close: *mut AnyObject = msg_send![ns_window, standardWindowButton: 0usize];
        let zoom: *mut AnyObject = msg_send![ns_window, standardWindowButton: 2usize];
        if close.is_null() || zoom.is_null() {
            return None;
        }
        // In window coordinates, from the bottom left corner
        let frame = |button: *mut AnyObject| -> NSRect {
            let bounds: NSRect = msg_send![button, bounds];
            msg_send![button, convertRect: bounds, toView: std::ptr::null_mut::<AnyObject>()]
        };
        let (close, zoom) = (frame(close), frame(zoom));
        let window_frame: NSRect = msg_send![ns_window, frame];
        Some(TrafficLights {
            left: close.origin.x,
            right: zoom.origin.x + zoom.size.width,
            top: window_frame.size.height - close.origin.y - close.size.height,
            height: close.size.height,
        })
    }
}
