//! Windows 11 Snap Layouts on the maximize button drawn by the drag region script.
//!
//! Windows opens the Snap Layouts flyout when a window answers `HTMAXBUTTON` to `WM_NCHITTEST`,
//! but the WebView2 child window covers the page and gets the hit tests. An invisible child
//! window (no redirection surface, so nothing is drawn) sits over the maximize button, above the
//! WebView2, and answers `HTMAXBUTTON` for it. The page no longer gets the mouse there, so the
//! child window reports hover and press to the script, which shows them on the button, and turns
//! the click into `ToggleMaximize`.

use std::sync::Once;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::commands::RustinoCommand;
use crate::window::CommandProxy;
use crate::window_ext::WindowCommand;

const CLASS_NAME: windows::core::PCWSTR = w!("RustinoSnapButton");

/// What the mouse does on the maximize button, which the script shows on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapButtonState {
    None,
    Hover,
    Pressed,
}

impl SnapButtonState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Hover => "hover",
            Self::Pressed => "pressed",
        }
    }
}

/// The child window over the maximize button.
pub struct SnapButton {
    hwnd: HWND,
}

/// Per-window data of the child window, in `GWLP_USERDATA`.
struct Data {
    proxy: CommandProxy,
    state: SnapButtonState,
}

impl SnapButton {
    pub fn new(parent: isize, proxy: CommandProxy) -> Option<Self> {
        static REGISTER: Once = Once::new();
        let instance = HINSTANCE::default();
        REGISTER.call_once(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance,
                lpszClassName: CLASS_NAME,
                hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
                ..Default::default()
            };
            unsafe { RegisterClassW(&class) };
        });
        let data = Box::into_raw(Box::new(Data { proxy, state: SnapButtonState::None }));
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP,
                CLASS_NAME,
                None,
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(HWND(parent as _)),
                None,
                Some(instance),
                Some(data as _),
            )
        };
        match hwnd {
            Ok(hwnd) => Some(Self { hwnd }),
            Err(_) => {
                drop(unsafe { Box::from_raw(data) });
                None
            }
        }
    }

    /// Places the child window over the button, in physical pixels of the client area, above the
    /// WebView2, or hides it.
    pub fn set_rect(&self, rect: Option<[i32; 4]>) {
        unsafe {
            match rect {
                Some([x, y, width, height]) if width > 0 && height > 0 => {
                    let _ = SetWindowPos(
                        self.hwnd,
                        Some(HWND_TOP),
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
                _ => {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
            }
        }
    }
}

impl Drop for SnapButton {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

fn data<'a>(hwnd: HWND) -> Option<&'a mut Data> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Data;
    unsafe { ptr.as_mut() }
}

fn set_state(hwnd: HWND, state: SnapButtonState) {
    if let Some(data) = data(hwnd)
        && data.state != state
    {
        data.state = state;
        let _ = data
            .proxy
            .send_event(RustinoCommand::Window(WindowCommand::SnapButtonState(state)));
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
        }
        WM_NCDESTROY => {
            let ptr = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut Data;
            if !ptr.is_null() {
                drop(unsafe { Box::from_raw(ptr) });
            }
        }
        // The whole window is the maximize button: Windows shows the Snap Layouts flyout on hover
        WM_NCHITTEST => return LRESULT(HTMAXBUTTON as isize),
        WM_NCMOUSEMOVE => {
            if data(hwnd).is_some_and(|data| data.state == SnapButtonState::None) {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = unsafe { TrackMouseEvent(&mut track) };
                set_state(hwnd, SnapButtonState::Hover);
            }
            // On to DefWindowProc, as Windows Terminal does: the Snap Layouts hover handling
        }
        WM_NCMOUSELEAVE => {
            set_state(hwnd, SnapButtonState::None);
            return LRESULT(0);
        }
        // Not to DefWindowProc, which would track the button of this window
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
            set_state(hwnd, SnapButtonState::Pressed);
            return LRESULT(0);
        }
        WM_NCLBUTTONUP => {
            if data(hwnd).is_some_and(|data| data.state == SnapButtonState::Pressed) {
                set_state(hwnd, SnapButtonState::Hover);
                if let Some(data) = data(hwnd) {
                    let _ = data.proxy.send_event(RustinoCommand::Window(WindowCommand::ToggleMaximize));
                }
            }
            return LRESULT(0);
        }
        // No system menu or other caption behavior from this window
        WM_NCRBUTTONDOWN | WM_NCRBUTTONUP => return LRESULT(0),
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
