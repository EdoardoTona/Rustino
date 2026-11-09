//! Windows menu accelerators.
//!
//! muda only builds the accelerator table: key presses must be translated by the host.
//! They reach the message loop while the host window has focus, and WebView2's
//! `AcceleratorKeyPressed` event while the webview has focus.

use std::cell::RefCell;

use webview2_com::AcceleratorKeyPressedEventHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, HACCEL, MSG, TranslateAcceleratorW, WM_KEYDOWN, WM_SYSKEYDOWN,
};
use wry::WebViewExtWindows;

thread_local! {
    // (window HWND, menu HACCEL, edit shortcuts of custom items) of the menu bars of the windows
    // whose event loop runs on this thread
    static MENUS: RefCell<Vec<(isize, isize, u8)>> = const { RefCell::new(Vec::new()) };
}

/// Sets the menu bar of a window, or removes it with `None`.
/// `custom_edit_shortcuts`: see `BuiltMenu::custom_edit_shortcuts`.
pub fn set_menu(hwnd: isize, menu: Option<&muda::Menu>, custom_edit_shortcuts: u8) {
    MENUS.with_borrow_mut(|menus| {
        menus.retain(|(window, _, _)| *window != hwnd);
        if let Some(menu) = menu {
            menus.push((hwnd, menu.haccel(), custom_edit_shortcuts));
        }
    });
}

/// Returns true when `msg` activated a menu item (the message must then be dropped).
pub fn translate(msg: &MSG) -> bool {
    let root = unsafe { GetAncestor(msg.hwnd, GA_ROOT) }.0 as isize;
    // Copied out: the menu item handlers may change the menus
    let Some((hwnd, haccel, custom_edit_shortcuts)) =
        MENUS.with_borrow(|menus| menus.iter().find(|(window, _, _)| *window == root).copied())
    else {
        return false;
    };
    // Ctrl+C/X/V/A/Z/Y are left to the focused control, unless a custom item uses them: muda's
    // predefined edit items would re-send the same keys, looping forever
    if haccel == 0 || edit_shortcut(msg) & !custom_edit_shortcuts != 0 {
        return false;
    }
    unsafe { TranslateAcceleratorW(HWND(hwnd as _), HACCEL(haccel as _), msg) != 0 }
}

/// The bit of `menu::EDIT_SHORTCUT_KEYS` for exactly Ctrl+C/X/V/A/Z/Y, 0 for other keys.
fn edit_shortcut(msg: &MSG) -> u8 {
    let pressed = |key: VIRTUAL_KEY| unsafe { GetKeyState(key.0 as i32) } < 0;
    if msg.message != WM_KEYDOWN
        || !pressed(VK_CONTROL)
        || [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN].into_iter().any(pressed)
    {
        return 0;
    }
    crate::menu::edit_shortcut_bit(msg.wParam.0 as u8)
}

/// `hwnd`: the window of the webview.
pub fn attach_webview(webview: &wry::WebView, hwnd: isize) {
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let (mut key, mut lparam) = (0u32, 0i32);
        unsafe {
            args.KeyEventKind(&mut kind)?;
            args.VirtualKey(&mut key)?;
            args.KeyEventLParam(&mut lparam)?;
        }
        let message = match kind {
            COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN => WM_KEYDOWN,
            COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN => WM_SYSKEYDOWN,
            _ => return Ok(()),
        };
        let msg = MSG {
            hwnd: HWND(hwnd as _),
            message,
            wParam: WPARAM(key as usize),
            lParam: LPARAM(lparam as isize),
            ..Default::default()
        };
        if translate(&msg) {
            unsafe { args.SetHandled(true)? };
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe {
        let _ = webview
            .controller()
            .add_AcceleratorKeyPressed(&handler, &mut token);
    }
}
