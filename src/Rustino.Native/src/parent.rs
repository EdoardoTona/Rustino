//! Child windows: a window with a parent stays above it and closes with it. A modal one also
//! blocks it until it closes: on Windows the parent is disabled, on macOS the window is a sheet of
//! the parent, on Linux a modal GTK window (which blocks the other windows of the app too).

use std::rc::{Rc, Weak};

use tao::window::{Window, WindowBuilder};

#[cfg(target_os = "windows")]
thread_local! {
    /// The open modal windows of each parent: the parent is enabled again when the last one closes
    static MODALS: std::cell::RefCell<std::collections::HashMap<tao::window::WindowId, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Makes the window being built a child of `parent`.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn configure(builder: WindowBuilder, parent: &Window, modal: bool) -> WindowBuilder {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
        // Owned, not a child window: it stays a top-level window above its owner
        builder.with_owner_window(parent.hwnd())
    }
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS};
        // A sheet has no parent window: `attach` begins it
        if modal { builder } else { builder.with_parent_window(parent.ns_window()) }
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::{WindowBuilderExtUnix, WindowExtUnix};
        builder.with_transient_for(parent.gtk_window())
    }
}

/// The link between a built window and its parent, undone when dropped, before the window.
pub struct Child {
    parent: Weak<Window>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    window: Weak<Window>,
    modal: bool,
}

pub fn attach(window: &Rc<Window>, parent: &Rc<Window>, modal: bool) -> Child {
    if modal {
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowExtWindows;
            MODALS.with(|m| *m.borrow_mut().entry(parent.id()).or_default() += 1);
            parent.set_enable(false);
        }
        #[cfg(target_os = "macos")]
        {
            use objc2::runtime::AnyObject;
            use tao::platform::macos::WindowExtMacOS;
            let (parent, sheet) = (parent.ns_window() as *mut AnyObject, window.ns_window() as *mut AnyObject);
            let no_handler: *mut AnyObject = std::ptr::null_mut();
            unsafe {
                let _: () = objc2::msg_send![parent, beginSheet: sheet, completionHandler: no_handler];
            }
        }
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::GtkWindowExt;
            use tao::platform::unix::WindowExtUnix;
            window.gtk_window().set_modal(true);
        }
    }
    Child {
        parent: Rc::downgrade(parent),
        window: Rc::downgrade(window),
        modal,
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let Some(parent) = self.parent.upgrade() else {
            return;
        };
        if !self.modal {
            return;
        }
        // Before the window closes, so that the parent gets the focus back
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowExtWindows;
            let last = MODALS
                .try_with(|m| {
                    let mut modals = m.borrow_mut();
                    let open = modals.entry(parent.id()).or_default();
                    *open = open.saturating_sub(1);
                    let last = *open == 0;
                    if last {
                        modals.remove(&parent.id());
                    }
                    last
                })
                .unwrap_or(true);
            if last {
                parent.set_enable(true);
                parent.set_focus();
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.upgrade() {
            use objc2::runtime::AnyObject;
            use tao::platform::macos::WindowExtMacOS;
            let (parent, sheet) = (parent.ns_window() as *mut AnyObject, window.ns_window() as *mut AnyObject);
            unsafe {
                let _: () = objc2::msg_send![parent, endSheet: sheet];
            }
        }
        #[cfg(target_os = "linux")]
        let _ = parent;
    }
}
