//! The title bar of macOS: its styles, and the traffic lights under the overlay title bar.

use std::cell::Cell;

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
    /// Keeps the lights where Rustino put them while the window resizes
    resize_observer: Option<Retained<ResizeObserver>>,
    proxy: CommandProxy,
    /// Checks in a row that found the lights moved back
    checks: u8,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class doesn't implement `Drop`
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RustinoTrafficLightsObserver"]
    // tao's inset of the lights, while Rustino places them
    #[ivars = Cell<Option<(f64, f64)>>]
    struct ResizeObserver;

    impl ResizeObserver {
        // AppKit puts the lights back at each step of a resize. The event loop moves them again
        // on the `Resized` event, but it doesn't run during an animated resize (zoom, double
        // click on the title bar): the lights would stay in their default place until its end.
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &AnyObject) {
            let Some((x, y)) = self.ivars().get() else {
                return;
            };
            let ns_window: *mut AnyObject = unsafe { msg_send![notification, object] };
            if !ns_window.is_null() && !has_fullscreen_mask(ns_window) {
                unsafe {
                    layout_title_bar(ns_window);
                    move_traffic_lights(ns_window, x, y);
                }
            }
        }
    }
);

impl ResizeObserver {
    /// Observes the resizes of the window, until dropped by `MacTitleBar`.
    fn new(ns_window: *mut AnyObject, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(None));
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let name = objc2_foundation::NSString::from_str("NSWindowDidResizeNotification");
        unsafe {
            let center: *mut AnyObject = msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![
                center,
                addObserver: &*this,
                selector: sel!(windowDidResize:),
                name: &*name,
                object: ns_window,
            ];
        }
        this
    }
}

impl Drop for MacTitleBar {
    fn drop(&mut self) {
        if let Some(observer) = self.resize_observer.take() {
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
    pub(super) fn new(style: MacTitleBarStyle, traffic_lights: Option<(f64, f64)>, proxy: CommandProxy) -> Self {
        Self {
            style,
            traffic_lights,
            pending: false,
            default_lights: None,
            placed: None,
            resize_observer: None,
            proxy,
            checks: 0,
        }
    }

    /// Records where the lights are, also for the resizes that follow.
    fn set_placed(&mut self, window: &Window, placed: Option<((f64, f64), (f64, f64))>) {
        use tao::platform::macos::WindowExtMacOS;
        self.placed = placed;
        if self.resize_observer.is_none()
            && placed.is_some()
            && let Some(mtm) = MainThreadMarker::new()
        {
            self.resize_observer = Some(ResizeObserver::new(window.ns_window().cast(), mtm));
        }
        if let Some(observer) = &self.resize_observer {
            observer.ivars().set(placed.map(|(inset, _)| inset));
        }
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
        if self.pending || in_fullscreen(window) {
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
            self.set_placed(window, placed);
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
                    self.set_placed(window, None);
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
            self.set_placed(window, height.map(|_| ((x, y), target)));
        }
        // AppKit lays out the title bar again after a change of style or decorations, later
        // and without telling: check once it's done, a few times at most
        if self.checks < 3 {
            self.checks += 1;
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let _ = proxy.send_event(RustinoCommand::Window(WindowCommand::CheckTrafficLights));
            });
        }
    }
}

/// Bounds tao's y for the traffic lights, against measures taken while AppKit moves them.
const MAX_TRAFFIC_LIGHT_INSET: f64 = 200.0;

/// Also while the window goes in and out of fullscreen, when tao no longer or not yet reports it.
fn in_fullscreen(window: &Window) -> bool {
    use tao::platform::macos::WindowExtMacOS;
    window.fullscreen().is_some() || has_fullscreen_mask(window.ns_window().cast())
}

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
