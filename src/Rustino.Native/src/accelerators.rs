//! Windows menu accelerators.
//!
//! muda only builds the accelerator table: key presses must be translated by the host.
//! They reach the message loop while the host window has focus, and WebView2's
//! `AcceleratorKeyPressed` event while the webview has focus.

use std::cell::Cell;

use webview2_com::AcceleratorKeyPressedEventHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, HACCEL, MSG, TranslateAcceleratorW, WM_KEYDOWN, WM_SYSKEYDOWN,
};
use wry::WebViewExtWindows;

thread_local! {
    // (window HWND, menu HACCEL) of the menu bar owned by this event loop thread
    static MENU: Cell<(isize, isize)> = const { Cell::new((0, 0)) };
}

pub fn set_menu(hwnd: isize, menu: Option<&muda::Menu>) {
    MENU.set((hwnd, menu.map_or(0, |m| m.haccel())));
}

/// Returns true when `msg` activated a menu item (the message must then be dropped).
pub fn translate(msg: &MSG) -> bool {
    let (hwnd, haccel) = MENU.get();
    if haccel == 0 || is_edit_shortcut(msg) {
        return false;
    }
    let hwnd = HWND(hwnd as _);
    unsafe {
        GetAncestor(msg.hwnd, GA_ROOT) == hwnd
            && TranslateAcceleratorW(hwnd, HACCEL(haccel as _), msg) != 0
    }
}

/// Ctrl+C/X/V/A/Z/Y are left to the focused control: muda's predefined edit items would
/// re-send the same keys, looping forever.
fn is_edit_shortcut(msg: &MSG) -> bool {
    msg.message == WM_KEYDOWN
        && matches!(msg.wParam.0 as u8, b'C' | b'X' | b'V' | b'A' | b'Z' | b'Y')
        && unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0
}

pub fn attach_webview(webview: &wry::WebView) {
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(|_, args| {
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
            hwnd: HWND(MENU.get().0 as _),
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
