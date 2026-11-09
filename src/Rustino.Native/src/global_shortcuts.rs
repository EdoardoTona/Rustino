//! Global shortcuts: key combinations that reach the app while another app has the focus. Each
//! window registers its own and gets their events; they're unregistered when it closes.
//!
//! Everything runs on the event loop thread: on Windows the manager gets the key presses through
//! a window of the thread that creates it, on macOS through the main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;
use std::sync::Once;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::commands::RustinoCommand;
use crate::util;
use crate::window::{RustinoWindow, configure};

// ---------------------------------------------------------------------------
// C API
// ---------------------------------------------------------------------------

/// Registers a global shortcut for the window. 1 when registered; before the window runs, 1 when
/// the accelerator is valid: the window registers it when it starts (failures go to the log). 0
/// when it's invalid, another window or app has it, or the window closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_register_global_shortcut(
    instance: *mut RustinoWindow,
    accelerator: *const c_char,
) -> i32 {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (Some(inst), Some(accelerator)) = (unsafe { instance.as_ref() }, unsafe { util::cstr_to_string(accelerator) }) else {
            return 0;
        };
        let Ok(hotkey) = parse(&accelerator) else {
            return 0;
        };
        let added = inst.configure(|s| {
            let shortcuts = &mut s.config.global_shortcuts;
            if !shortcuts.iter().any(|a| parse(a).is_ok_and(|h| h.id() == hotkey.id())) {
                shortcuts.push(accelerator.clone());
            }
        });
        if added {
            return 1;
        }
        let key = inst.key();
        let register_now = {
            let accelerator = accelerator.clone();
            move |_: &tao::window::Window, _: &wry::WebView| register(key, &accelerator)
        };
        match inst.invoke(register_now) {
            Some(result) => i32::from(result.is_ok()),
            // Starting: registered once it runs
            None if !inst.is_running() => i32::from(inst.post(move |_, _| {
                let _ = register(key, &accelerator);
            })),
            None => 0,
        }
    }))
    .unwrap_or(0)
}

/// 1 when the window had registered the shortcut (before it runs: when it was to register it).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_unregister_global_shortcut(
    instance: *mut RustinoWindow,
    accelerator: *const c_char,
) -> i32 {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (Some(inst), Some(accelerator)) = (unsafe { instance.as_ref() }, unsafe { util::cstr_to_string(accelerator) }) else {
            return 0;
        };
        let Ok(hotkey) = parse(&accelerator) else {
            return 0;
        };
        let mut removed = false;
        if inst.configure(|s| {
            let shortcuts = &mut s.config.global_shortcuts;
            let before = shortcuts.len();
            shortcuts.retain(|a| parse(a).map_or(true, |h| h.id() != hotkey.id()));
            removed = shortcuts.len() != before;
        }) {
            return i32::from(removed);
        }
        let key = inst.key();
        let unregister_now = {
            let accelerator = accelerator.clone();
            move |_: &tao::window::Window, _: &wry::WebView| unregister(key, &accelerator)
        };
        match inst.invoke(unregister_now) {
            Some(unregistered) => i32::from(unregistered),
            None if !inst.is_running() => i32::from(inst.post(move |_, _| {
                unregister(key, &accelerator);
            })),
            None => 0,
        }
    }))
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_unregister_all_global_shortcuts(instance: *mut RustinoWindow) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if inst.configure(|s| s.config.global_shortcuts.clear()) {
                return;
            }
            let key = inst.key();
            inst.post(move |_, _| unregister_all(key));
        }
    }));
}

/// 1 when the running window registered the shortcut.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_is_global_shortcut_registered(
    instance: *mut RustinoWindow,
    accelerator: *const c_char,
) -> i32 {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (Some(inst), Some(accelerator)) = (unsafe { instance.as_ref() }, unsafe { util::cstr_to_string(accelerator) }) else {
            return 0;
        };
        let key = inst.key();
        let registered = inst.invoke(move |_, _| is_registered(key, &accelerator));
        i32::from(registered == Some(true))
    }))
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_global_shortcut_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_global_shortcut = handler) };
}

// ---------------------------------------------------------------------------
// Shortcuts of the windows, on the event loop thread
// ---------------------------------------------------------------------------

struct Registered {
    window: usize,
    hotkey: HotKey,
    /// As the host registered it, for its events
    accelerator: String,
}

thread_local! {
    static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
    /// By hotkey id
    static REGISTERED: RefCell<HashMap<u32, Registered>> = RefCell::new(HashMap::new());
}

/// Parses an accelerator such as `CmdOrCtrl+Shift+K` (the syntax of the menu accelerators).
pub fn parse(accelerator: &str) -> Result<HotKey, String> {
    accelerator
        .parse::<HotKey>()
        .map_err(|e| format!("Invalid shortcut \"{accelerator}\": {e}"))
}

/// Registers the shortcut for the window `window`. Registering it again from the same window
/// does nothing; it fails when another window or app has it.
pub fn register(window: usize, accelerator: &str) -> Result<(), String> {
    let hotkey = parse(accelerator)?;
    if let Some(owner) = REGISTERED.with(|r| r.borrow().get(&hotkey.id()).map(|r| r.window)) {
        return if owner == window {
            Ok(())
        } else {
            Err(format!("The shortcut \"{accelerator}\" is registered by another window."))
        };
    }
    with_manager(|manager| manager.register(hotkey))?
        .map_err(|e| format!("Failed to register the shortcut \"{accelerator}\": {e}"))?;
    REGISTERED.with(|r| {
        r.borrow_mut().insert(
            hotkey.id(),
            Registered {
                window,
                hotkey,
                accelerator: accelerator.to_string(),
            },
        )
    });
    Ok(())
}

/// Returns false when the window hasn't registered the shortcut.
pub fn unregister(window: usize, accelerator: &str) -> bool {
    let Ok(hotkey) = parse(accelerator) else {
        return false;
    };
    let removed = REGISTERED.with(|r| {
        let mut registered = r.borrow_mut();
        match registered.get(&hotkey.id()) {
            Some(r) if r.window == window => registered.remove(&hotkey.id()),
            _ => None,
        }
    });
    match removed {
        Some(r) => {
            let _ = with_manager(|manager| manager.unregister(r.hotkey));
            true
        }
        None => false,
    }
}

pub fn unregister_all(window: usize) {
    // The thread-locals may be gone when the thread exits
    let removed: Vec<HotKey> = REGISTERED
        .try_with(|r| {
            let mut registered = r.borrow_mut();
            let ids: Vec<u32> = registered.iter().filter(|(_, r)| r.window == window).map(|(id, _)| *id).collect();
            ids.iter().filter_map(|id| registered.remove(id)).map(|r| r.hotkey).collect()
        })
        .unwrap_or_default();
    if !removed.is_empty() {
        let _ = with_manager(|manager| manager.unregister_all(&removed));
    }
}

pub fn is_registered(window: usize, accelerator: &str) -> bool {
    parse(accelerator).is_ok_and(|hotkey| {
        REGISTERED.with(|r| r.borrow().get(&hotkey.id()).is_some_and(|r| r.window == window))
    })
}

/// The accelerator of the shortcut `id` when the window `window` registered it.
pub fn accelerator(window: usize, id: u32) -> Option<String> {
    REGISTERED.with(|r| {
        r.borrow()
            .get(&id)
            .filter(|r| r.window == window)
            .map(|r| r.accelerator.clone())
    })
}

fn with_manager<R>(f: impl FnOnce(&GlobalHotKeyManager) -> R) -> Result<R, String> {
    static INSTALL_HANDLER: Once = Once::new();
    // global-hotkey keeps one event handler for the whole process: it forwards the presses to
    // every running window, which ignores the shortcuts of the others
    INSTALL_HANDLER.call_once(|| {
        GlobalHotKeyEvent::set_event_handler(Some(|event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                crate::window::send_to_event_loops(|| RustinoCommand::GlobalShortcutPressed(event.id()));
            }
        }));
    });
    MANAGER
        .try_with(|manager| {
            let mut manager = manager.borrow_mut();
            if manager.is_none() {
                // global-hotkey registers through X11 on another thread, and only logs its failures
                #[cfg(target_os = "linux")]
                if std::env::var_os("DISPLAY").is_none() {
                    return Err("Global shortcuts need an X11 display.".to_string());
                }
                *manager = Some(
                    GlobalHotKeyManager::new()
                        .map_err(|e| format!("Global shortcuts are unavailable: {e}"))?,
                );
            }
            Ok(f(manager.as_ref().expect("the manager was just created")))
        })
        .map_err(|_| "The event loop is closing.".to_string())?
}

/// Unregisters the shortcuts of the window when dropped.
pub struct Owner(pub usize);

impl Drop for Owner {
    fn drop(&mut self) {
        unregister_all(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_menu_accelerators() {
        assert!(parse("CmdOrCtrl+Shift+K").is_ok());
        assert!(parse("Alt+F4").is_ok());
        assert_eq!(parse("ctrl+k").unwrap().id(), parse("Control+K").unwrap().id());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Ctrl+Nope").is_err());
    }

    #[test]
    fn unknown_shortcuts_are_not_registered() {
        assert!(!is_registered(1, "Ctrl+K"));
        assert!(!unregister(1, "Ctrl+K"));
        assert_eq!(accelerator(1, parse("Ctrl+K").unwrap().id()), None);
    }
}
