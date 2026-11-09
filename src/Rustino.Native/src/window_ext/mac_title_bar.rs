//! The title bar of macOS: its styles, and the traffic lights under the overlay title bar.

use tao::event::{Event, WindowEvent};
use tao::window::Window;

use crate::commands::RustinoCommand;
use crate::window::CommandProxy;

use super::{MacTitleBarStyle, WindowCommand, WindowControls};

/// The title bar style while the window runs.
pub(super) struct MacTitleBar {
    pub(super) style: MacTitleBarStyle,
    pub(super) traffic_lights: Option<(f64, f64)>,
    /// The decorations are coming back: tao rebuilds the style mask asynchronously, without the
    /// style, which goes back on the resize that follows
    pub(super) pending: bool,
    /// Where the traffic lights sit before Rustino moves them: they go back there when the
    /// overlay title bar loses its height
    pub(super) default_lights: Option<TrafficLights>,
    /// Where Rustino put the traffic lights: tao's inset and the left and top margins it gives
    pub(super) placed: Option<((f64, f64), (f64, f64))>,
    pub(super) proxy: CommandProxy,
    /// Checks in a row that found the lights moved back
    pub(super) checks: u8,
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
        let ns_window = window.ns_window() as *mut objc2::runtime::AnyObject;
        let visibility: isize = if overlay { 1 } else { 0 };
        unsafe {
            let _: () = objc2::msg_send![ns_window, setTitleVisibility: visibility];
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
        // AppKit puts the lights back in their place when the decorations come back, in a layout
        // pass that runs only after the resize: run it now
        let ns_window = window.ns_window() as *mut objc2::runtime::AnyObject;
        unsafe {
            let content: *mut objc2::runtime::AnyObject = objc2::msg_send![ns_window, contentView];
            let frame_view: *mut objc2::runtime::AnyObject = objc2::msg_send![content, superview];
            if !frame_view.is_null() {
                let _: () = objc2::msg_send![frame_view, layoutSubtreeIfNeeded];
            }
        }
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
            self.placed = traffic_lights(window).map(|lights| ((x, y), (lights.left, lights.top)));
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
                    self.placed = None;
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
            self.placed = height.map(|_| ((x, y), target));
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
    // NSWindowStyleMaskFullScreen
    const FULL_SCREEN: usize = 1 << 14;
    let ns_window = window.ns_window() as *mut objc2::runtime::AnyObject;
    let mask: usize = unsafe { objc2::msg_send![ns_window, styleMask] };
    window.fullscreen().is_some() || mask & FULL_SCREEN != 0
}

/// tao stores the inset for its `drawRect:`, which the webview over the whole window keeps from
/// running, and for leaving fullscreen: the lights move now as tao would move them.
fn set_traffic_light_inset_now(window: &Window, x: f64, y: f64) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSPoint, NSRect};
    use tao::platform::macos::WindowExtMacOS;
    window.set_traffic_light_inset(tao::dpi::LogicalPosition::new(x, y));
    let ns_window = window.ns_window() as *mut AnyObject;
    unsafe {
        // NSWindowCloseButton, NSWindowMiniaturizeButton and NSWindowZoomButton
        let buttons: [*mut AnyObject; 3] = [
            objc2::msg_send![ns_window, standardWindowButton: 0usize],
            objc2::msg_send![ns_window, standardWindowButton: 1usize],
            objc2::msg_send![ns_window, standardWindowButton: 2usize],
        ];
        if buttons.iter().any(|button| button.is_null()) {
            return;
        }
        let close: NSRect = objc2::msg_send![buttons[0], frame];
        let miniaturize: NSRect = objc2::msg_send![buttons[1], frame];
        let titlebar: *mut AnyObject = objc2::msg_send![buttons[0], superview];
        let container: *mut AnyObject = objc2::msg_send![titlebar, superview];
        if container.is_null() {
            return;
        }
        let window_frame: NSRect = objc2::msg_send![ns_window, frame];
        let mut container_frame: NSRect = objc2::msg_send![container, frame];
        container_frame.size.height = close.size.height + y;
        container_frame.origin.y = window_frame.size.height - container_frame.size.height;
        let _: () = objc2::msg_send![container, setFrame: container_frame];
        let spacing = miniaturize.origin.x - close.origin.x;
        for (i, &button) in buttons.iter().enumerate() {
            let frame: NSRect = objc2::msg_send![button, frame];
            let origin = NSPoint::new(x + i as f64 * spacing, frame.origin.y);
            let _: () = objc2::msg_send![button, setFrameOrigin: origin];
        }
    }
}

/// `None` when the window has no traffic lights (chromeless).
fn traffic_lights(window: &Window) -> Option<TrafficLights> {
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSRect;
    use tao::platform::macos::WindowExtMacOS;
    let ns_window = window.ns_window() as *mut AnyObject;
    unsafe {
        // NSWindowCloseButton and NSWindowZoomButton
        let close: *mut AnyObject = objc2::msg_send![ns_window, standardWindowButton: 0usize];
        let zoom: *mut AnyObject = objc2::msg_send![ns_window, standardWindowButton: 2usize];
        if close.is_null() || zoom.is_null() {
            return None;
        }
        // In window coordinates, from the bottom left corner
        let frame = |button: *mut AnyObject| -> NSRect {
            let bounds: NSRect = objc2::msg_send![button, bounds];
            objc2::msg_send![button, convertRect: bounds, toView: std::ptr::null_mut::<AnyObject>()]
        };
        let (close, zoom) = (frame(close), frame(zoom));
        let window_frame: NSRect = objc2::msg_send![ns_window, frame];
        Some(TrafficLights {
            left: close.origin.x,
            right: zoom.origin.x + zoom.size.width,
            top: window_frame.size.height - close.origin.y - close.size.height,
            height: close.size.height,
        })
    }
}
