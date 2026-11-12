//! Kiosk mode: the window covers its screen, stays on top and in front, and the user can't close
//! it, minimize it or switch to another app. Only the host leaves it (`SetKiosk(false)`, or by
//! closing the window from code).
//!
//! - Windows: borderless fullscreen and always on top; a low-level keyboard hook swallows the
//!   shortcuts of the system (Windows key, Alt+Tab, Alt+Esc, Ctrl+Esc, Ctrl+Shift+Esc, Alt+F4,
//!   Alt+Space), PrintScreen, Search/Home, application launch keys and Alt+Shift+NumLock.
//!   Accessibility activation shortcuts and edge swipes of touch screens are off. Touchpad gestures with three
//!   and four fingers aren't keys and stay (`TouchpadGesturesController`, tried on build 26300,
//!   turns on for a desktop app but gets none of them): the window comes back after them,
//!   unless the host asks to turn them off in the settings of the user meanwhile. Ctrl+Alt+Del and Win+L can't be blocked
//!   by an app. The raw keyboard input of tao is off meanwhile: Windows doesn't call the keyboard
//!   hook of a process for the keys it gets as raw input.
//! - macOS: the presentation options AppKit has for kiosks (no Dock, menu bar, app switching,
//!   Force Quit, hiding, logout) over a window as large as the screen, and Cmd+Q doesn't quit.
//! - Linux: fullscreen and above the others, with the keyboard grabbed: on X11 the window manager
//!   doesn't see its shortcuts, on Wayland the compositor is asked to inhibit them.
//!
//! Window and keyboard locks end with the process. Windows accessibility shortcut settings
//! are restored after the last kiosk window exits; a later kiosk recovers the saved settings
//! after a crash. Turning off touchpad gestures is a separate, explicit change to user settings.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::EventLoopWindowTarget;
use tao::window::Window;

use super::WindowCommand;
use crate::commands::RustinoCommand;
use crate::state::SharedState;
use crate::window::{CommandProxy, Wake};

/// How often the window takes back what it lost (the foreground, the fullscreen, the keyboard):
/// the system tells little of it with events.
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

pub struct Kiosk {
    proxy: CommandProxy,
    state: Arc<SharedState>,
    /// Always on top as the host asked it: the window goes back to it
    topmost: bool,
    /// Asked before the window ran (with or without the touchpad gestures): starts with the
    /// event loop
    start: Option<bool>,
    active: Option<Active>,
}

struct Active {
    /// Fullscreen as the host asked it: the window goes back to it
    fullscreen: bool,
    lock: platform::Lock,
    _checks: Checks,
}

impl Kiosk {
    pub fn new(proxy: CommandProxy, state: Arc<SharedState>, topmost: bool, start: Option<bool>) -> Self {
        Self {
            proxy,
            state,
            topmost,
            start,
            active: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// `without_touchpad_gestures`: Windows only, when the mode turns on.
    pub fn set(
        &mut self,
        on: bool,
        without_touchpad_gestures: bool,
        window: &Window,
        #[allow(unused_variables)] target: &EventLoopWindowTarget<Wake>,
    ) {
        if on == self.is_active() {
            return;
        }
        self.state.is_kiosk.store(on, Ordering::Release);
        if on {
            self.active = Some(Active {
                fullscreen: window.fullscreen().is_some(),
                lock: platform::Lock::new(window, without_touchpad_gestures),
                _checks: Checks::start(self.proxy.clone()),
            });
            #[cfg(target_os = "windows")]
            platform::set_raw_input(target, false);
            window.set_always_on_top(true);
            self.check(window);
        } else if let Some(mut active) = self.active.take() {
            #[cfg(target_os = "windows")]
            platform::set_raw_input(target, true);
            active.lock.release(window, active.fullscreen);
            window.set_always_on_top(self.topmost);
        }
    }

    /// Takes back what the window lost.
    pub fn check(&mut self, window: &Window) {
        let Some(active) = &mut self.active else { return };
        // Hidden by the host: nothing to keep in front
        if !self.state.is_visible.load(Ordering::Acquire) {
            return;
        }
        if window.is_minimized() {
            window.set_minimized(false);
            self.state.is_minimized.store(false, Ordering::Release);
        }
        active.lock.hold(window);
    }

    /// Follows the events the kiosk depends on, and keeps the ones that would leave it: the
    /// requests to close the window and the commands that take it out of the fullscreen.
    pub fn filter<'a>(
        &mut self,
        event: Event<'a, RustinoCommand>,
        window: &Window,
        target: &EventLoopWindowTarget<Wake>,
    ) -> Option<Event<'a, RustinoCommand>> {
        match &event {
            Event::NewEvents(StartCause::Init) => {
                if let Some(without_touchpad_gestures) = self.start.take() {
                    self.set(true, without_touchpad_gestures, window, target);
                }
            }
            Event::UserEvent(RustinoCommand::SetTopmost(topmost)) => self.topmost = *topmost,
            _ => {}
        }
        let Some(active) = &mut self.active else {
            return Some(event);
        };
        match &event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => return None,
            // Applied when the kiosk ends
            Event::UserEvent(RustinoCommand::SetFullscreen(fullscreen)) => {
                active.fullscreen = *fullscreen;
                return None;
            }
            Event::UserEvent(
                RustinoCommand::SetTopmost(_)
                | RustinoCommand::SetMinimized(true)
                | RustinoCommand::Window(
                    WindowCommand::Minimize
                    | WindowCommand::RequestClose
                    | WindowCommand::ToggleMaximize
                    | WindowCommand::DragWindow
                    | WindowCommand::DragResizeWindow(_),
                ),
            ) => return None,
            // Minimized or out of the fullscreen
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => self.check(window),
            _ => {}
        }
        Some(event)
    }
}

/// Asks the event loop for a check every `CHECK_INTERVAL`, until dropped.
struct Checks(#[allow(dead_code)] mpsc::Sender<()>);

impl Checks {
    fn start(proxy: CommandProxy) -> Self {
        let (stop, stopped) = mpsc::channel::<()>();
        let _ = std::thread::Builder::new().name("rustino-kiosk".into()).spawn(move || {
            // Disconnected once the kiosk ends
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(CHECK_INTERVAL) {
                if proxy.send_event(RustinoCommand::Window(WindowCommand::KioskCheck)).is_err() {
                    break;
                }
            }
        });
        Self(stop)
    }
}

/// Whether the kiosk swallows a key (Windows virtual key code): the Windows keys with all their
/// shortcuts, Alt+Tab, Alt+F4, Alt+Space (the window menu), Alt+Esc, Ctrl+Esc (Start) and
/// Ctrl+Shift+Esc (Task Manager), system/application launch keys, PrintScreen, Sleep and
/// Alt+Shift+NumLock. Shift activation sequences use accessibility APIs. The Sleep key is
/// swallowed for the keyboards that send it as a key: the ones wired to the power manager
/// don't come through here. F12 and Ctrl+Shift+I stay with the page: the developer tools
/// open only when the host enabled them.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_system_shortcut(key: u32, alt: bool, ctrl: bool, shift: bool) -> bool {
    const TAB: u32 = 0x09;
    const ESCAPE: u32 = 0x1B;
    const SPACE: u32 = 0x20;
    const PRINT_SCREEN: u32 = 0x2C;
    const LEFT_WIN: u32 = 0x5B;
    const RIGHT_WIN: u32 = 0x5C;
    const SLEEP: u32 = 0x5F;
    const F4: u32 = 0x73;
    const NUM_LOCK: u32 = 0x90;
    const BROWSER_SEARCH: u32 = 0xAA;
    const BROWSER_HOME: u32 = 0xAC;
    const LAUNCH_MAIL: u32 = 0xB4;
    const LAUNCH_MEDIA: u32 = 0xB5;
    const LAUNCH_APP1: u32 = 0xB6;
    const LAUNCH_APP2: u32 = 0xB7;
    match key {
        LEFT_WIN | RIGHT_WIN | PRINT_SCREEN | SLEEP | BROWSER_SEARCH | BROWSER_HOME | LAUNCH_MAIL
        | LAUNCH_MEDIA | LAUNCH_APP1 | LAUNCH_APP2 => true,
        TAB | F4 | SPACE => alt,
        ESCAPE => alt || ctrl,
        NUM_LOCK => alt && shift,
        _ => false,
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::sync::Mutex;
    use std::sync::mpsc;

    use tao::event_loop::{DeviceEventFilter, EventLoopWindowTarget};
    use tao::platform::windows::WindowExtWindows;
    use tao::window::{Fullscreen, Window};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, PROPERTYKEY, VARIANT_FALSE, VARIANT_TRUE, WPARAM};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
    use windows::Win32::System::Variant::VT_BOOL;
    use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VK_CONTROL, VK_ESCAPE, VK_SHIFT,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, CallNextHookEx, EVENT_SYSTEM_FOREGROUND, GA_ROOTOWNER, GetAncestor, GetClassNameW, GetForegroundWindow, GetMessageW,
        GetWindowThreadProcessId, HC_ACTION, KBDLLHOOKSTRUCT, KillTimer, LLKHF_ALTDOWN, MSG, PM_NOREMOVE,
        PeekMessageW, PostThreadMessageW, SetForegroundWindow, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
        WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT, WM_TIMER,
    };
    use windows::core::GUID;

    use crate::commands::RustinoCommand;
    use crate::window::Wake;
    use crate::window_ext::WindowCommand;

    pub struct Lock {
        _keys: KeyHook,
        _accessibility: crate::window_ext::kiosk_accessibility::Lock,
        /// Tells when another app takes the foreground
        foreground_events: HWINEVENTHOOK,
        /// The touchpad gestures of the user are off for this window
        without_touchpad_gestures: bool,
    }

    impl Lock {
        pub fn new(window: &Window, without_touchpad_gestures: bool) -> Self {
            set_edge_gestures(window.hwnd(), false);
            if without_touchpad_gestures {
                touchpad_gestures::turn_off();
            }
            // What the keyboard hook can't stop (the gestures of a touchpad that show the
            // desktop or another virtual desktop, an app that opens) is undone as it happens,
            // without waiting for the next check. The events come on this thread, the event
            // loop's.
            let foreground_events = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(foreground_changed),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            Self {
                _keys: KeyHook::new(),
                _accessibility: crate::window_ext::kiosk_accessibility::Lock::new(),
                foreground_events,
                without_touchpad_gestures,
            }
        }

        pub fn hold(&mut self, window: &Window) {
            if window.fullscreen().is_none() {
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }
            if !foreground_is_ours() {
                take_foreground(window);
            }
        }

        pub fn release(&mut self, window: &Window, fullscreen: bool) {
            set_edge_gestures(window.hwnd(), true);
            if !fullscreen {
                window.set_fullscreen(None);
            }
        }
    }

    impl Drop for Lock {
        fn drop(&mut self) {
            if !self.foreground_events.is_invalid() {
                let _ = unsafe { UnhookWinEvent(self.foreground_events) };
            }
            if self.without_touchpad_gestures {
                touchpad_gestures::restore();
            }
        }
    }

    fn class_of(hwnd: HWND) -> String {
        let mut name = [0u16; 128];
        let len = unsafe { GetClassNameW(hwnd, &mut name) };
        String::from_utf16_lossy(&name[..len.max(0) as usize])
    }

    /// The gestures of precision touchpads with three and four fingers (show the desktop, task
    /// view, switch app or virtual desktop, search, notification center) aren't keys and no API
    /// keeps them from the system: only the touchpad settings of the user do. Unlike the rest of
    /// the kiosk mode this changes the system, so the host asks for it: the values of the user
    /// are saved in the registry first, and put back when the last window leaves the kiosk mode.
    /// A process that ends before leaves the gestures off; the saved values are still there for
    /// the next time the kiosk mode ends.
    mod touchpad_gestures {
        use std::ffi::c_void;
        use std::sync::Mutex;

        use windows::Wdk::System::SystemServices::RtlGetVersion;
        use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, IServiceProvider};
        use windows::Win32::System::Registry::{
            HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW,
            RegSetKeyValueW,
        };
        use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
        use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface};

        const SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\PrecisionTouchPad";
        /// The values the user had, while the gestures are off
        const SAVED: &str = r"Software\Rustino\Kiosk\TouchpadGestures";
        /// Marks the saved values: the ones the user never set aren't there
        const SAVED_MARK: &str = "Saved";
        /// 0 is "Nothing" in Settings › Bluetooth & devices › Touchpad
        const GESTURES: [&str; 4] = [
            "ThreeFingerSlideEnabled",
            "ThreeFingerTapEnabled",
            "FourFingerSlideEnabled",
            "FourFingerTapEnabled",
        ];

        /// The windows that asked for it
        static WINDOWS: Mutex<usize> = Mutex::new(0);

        pub fn turn_off() {
            let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
            *windows += 1;
            if *windows > 1 {
                return;
            }
            // Saved by a process that ended in kiosk mode: those are the values of the user
            if read(SAVED, SAVED_MARK).is_none() {
                for gesture in GESTURES {
                    if let Some(value) = read(SETTINGS, gesture) {
                        write(SAVED, gesture, value);
                    }
                }
                write(SAVED, SAVED_MARK, 1);
            }
            for gesture in GESTURES {
                write(SETTINGS, gesture, 0);
            }
            if let Err(error) = notify() {
                eprintln!("Rustino kiosk: could not apply touchpad gesture settings: {error}");
            }
        }

        pub fn restore() {
            let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
            *windows = windows.saturating_sub(1);
            if *windows > 0 || read(SAVED, SAVED_MARK).is_none() {
                return;
            }
            for gesture in GESTURES {
                match read(SAVED, gesture) {
                    Some(value) => write(SETTINGS, gesture, value),
                    None => unsafe {
                        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(SETTINGS), &HSTRING::from(gesture));
                    },
                }
            }
            if let Err(error) = notify() {
                // Keep the recovery copy if Explorer did not apply the restored values.
                eprintln!("Rustino kiosk: could not apply restored touchpad gesture settings: {error}");
                return;
            }
            let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(SAVED)) };
        }

        fn read(key: &str, name: &str) -> Option<u32> {
            let (mut value, mut size) = (0u32, size_of::<u32>() as u32);
            unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(key),
                    &HSTRING::from(name),
                    RRF_RT_REG_DWORD,
                    None,
                    Some(&mut value as *mut u32 as *mut c_void),
                    Some(&mut size),
                )
            }
            .is_ok()
            .then_some(value)
        }

        fn write(key: &str, name: &str, value: u32) {
            unsafe {
                let _ = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(key),
                    &HSTRING::from(name),
                    REG_DWORD.0,
                    Some(&value as *const u32 as *const c_void),
                    size_of::<u32>() as u32,
                );
            }
        }

        // Explorer caches these four settings. WM_SETTINGCHANGE does not invalidate that cache;
        // use the same internal COM service as Settings, without restarting Explorer.
        // The interface is undocumented; the setting IDs changed between Windows releases.
        // Verified on ETXPS build 26300.9711: registry writes alone leave GetDWORD unchanged,
        // OnSettingChanged reloads all four values, including defaults after deleting a value.
        // Source: https://github.com/nvaccess/nvda/discussions/19841
        #[repr(transparent)]
        #[derive(Clone)]
        struct SettingsCache(IUnknown);

        // SAFETY: this is the IUnknown + OnSettingChanged prefix of IImmersiveSettingsCache.
        // Only that prefix is called; IUnknown owns and releases the COM reference.
        unsafe impl Interface for SettingsCache {
            type Vtable = SettingsCacheVtable;
            const IID: GUID = GUID::from_u128(0x4214f6fa_eb36_4e2f_9ca2_23fdc1832df7);
        }

        #[repr(C)] // abi_layout: not shared with .NET
        struct SettingsCacheVtable {
            base: IUnknown_Vtbl,
            on_setting_changed: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
        }

        fn notify() -> windows::core::Result<()> {
            const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
            const SETTINGS_CACHE: GUID = GUID::from_u128(0x53660488_8855_460b_a9ab_5cfc6b5012ca);
            // RtlGetVersion reports the actual build, regardless of the host app's manifest.
            let mut version = OSVERSIONINFOW {
                dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
                ..Default::default()
            };
            unsafe {
                RtlGetVersion(&mut version).ok()?;
                let first = if version.dwBuildNumber >= 22621 {
                    0x0F // Windows 11 22H2 and later
                } else if version.dwBuildNumber >= 22000 {
                    0x0E // Windows 11 21H2
                } else {
                    0x09 // Windows 10
                };
                let provider: IServiceProvider = CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER)?;
                let cache: SettingsCache = provider.QueryService(&SETTINGS_CACHE)?;
                for setting in first..first + 4 {
                    (cache.vtable().on_setting_changed)(cache.as_raw(), setting).ok()?;
                }
            }
            Ok(())
        }
    }

    /// A window of another process is in the foreground: the windows in kiosk mode check.
    unsafe extern "system" fn foreground_changed(
        _hook: HWINEVENTHOOK,
        _event: u32,
        _hwnd: HWND,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        crate::window::send_to_event_loops(|| RustinoCommand::Window(WindowCommand::KioskCheck));
    }

    /// Windows lets only the foreground process choose the foreground window: asked plainly
    /// first, then as part of the input of the foreground thread, then as tao does it (after an
    /// Alt key press of its own, which makes this process the one of the last input). The panels
    /// of the shell (Start, search, notification center, quick settings) give in to none of
    /// them: they close on Esc, and the next check takes the foreground they leave.
    fn take_foreground(window: &Window) {
        let hwnd = HWND(window.hwnd() as _);
        unsafe {
            let _ = SetForegroundWindow(hwnd);
            if GetForegroundWindow() == hwnd {
                return;
            }
            let (this_thread, foreground) = (GetCurrentThreadId(), GetForegroundWindow());
            let thread = GetWindowThreadProcessId(foreground, None);
            if thread != 0 && thread != this_thread && AttachThreadInput(this_thread, thread, true).as_bool() {
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
                let _ = AttachThreadInput(this_thread, thread, false);
            }
            if GetForegroundWindow() == hwnd {
                return;
            }
            window.set_focus();
            let foreground = GetForegroundWindow();
            if foreground != hwnd && class_of(foreground) == SHELL_PANEL_CLASS {
                let key = |flags| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_ESCAPE,
                            wScan: 1,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32);
            }
        }
    }

    /// The window class of the panels of the shell
    const SHELL_PANEL_CLASS: &str = "Windows.UI.Core.CoreWindow";

    /// Windows doesn't call the low-level keyboard hook of a process for the keys it gets as
    /// raw input, which tao asks for (for its device events) while one of its windows has the
    /// focus: off while a window is in kiosk mode.
    pub fn set_raw_input(target: &EventLoopWindowTarget<Wake>, on: bool) {
        let windows = KEY_HOOK.lock().unwrap_or_else(|e| e.into_inner()).windows;
        if !on {
            target.set_device_event_filter(DeviceEventFilter::Always);
        } else if windows <= 1 {
            // The last window leaves the kiosk mode: tao's default
            target.set_device_event_filter(DeviceEventFilter::Unfocused);
        }
    }

    /// Whether the foreground window is of this process, or owned by one of its windows (the
    /// dialogs of the webview, which are of its browser process).
    fn foreground_is_ours() -> bool {
        unsafe {
            let foreground = GetForegroundWindow();
            // Between two windows, or on the secure desktop: the next check tells
            if foreground.is_invalid() {
                return true;
            }
            let owner = GetAncestor(foreground, GA_ROOTOWNER);
            let mut process = 0u32;
            GetWindowThreadProcessId(if owner.is_invalid() { foreground } else { owner }, Some(&mut process));
            process == GetCurrentProcessId()
        }
    }

    /// The swipes from the edges of a touch screen (notification center, widgets, task view) while
    /// the window is fullscreen: the `System.EdgeGesture.DisableTouchWhenFullscreen` property.
    fn set_edge_gestures(hwnd: isize, enabled: bool) {
        const DISABLE_TOUCH_WHEN_FULLSCREEN: PROPERTYKEY = PROPERTYKEY {
            fmtid: GUID::from_u128(0x32ce38b2_2c9a_41b1_9bc5_b3784394aa44),
            pid: 2,
        };
        unsafe {
            let Ok(store) = SHGetPropertyStoreForWindow::<IPropertyStore>(HWND(hwnd as _)) else {
                return;
            };
            let mut value = PROPVARIANT::default();
            let variant = &mut *value.Anonymous.Anonymous;
            variant.vt = VT_BOOL;
            variant.Anonymous.boolVal = if enabled { VARIANT_FALSE } else { VARIANT_TRUE };
            let _ = store.SetValue(&DISABLE_TOUCH_WHEN_FULLSCREEN, &value);
        }
    }

    /// How often the keyboard hook is installed again
    const REINSTALL_MS: u32 = 1000;

    /// The keyboard hook of the process, while a window is in kiosk mode.
    struct KeyHook;

    #[derive(Default)]
    struct KeyHookState {
        windows: usize,
        /// The thread of the hook, which ends with a `WM_QUIT`
        thread: Option<u32>,
    }

    static KEY_HOOK: Mutex<KeyHookState> = Mutex::new(KeyHookState {
        windows: 0,
        thread: None,
    });

    impl KeyHook {
        fn new() -> Self {
            let mut state = KEY_HOOK.lock().unwrap_or_else(|e| e.into_inner());
            state.windows += 1;
            if state.windows == 1 {
                state.thread = start_key_hook();
            }
            Self
        }
    }

    impl Drop for KeyHook {
        fn drop(&mut self) {
            let mut state = KEY_HOOK.lock().unwrap_or_else(|e| e.into_inner());
            state.windows -= 1;
            if state.windows == 0
                && let Some(thread) = state.thread.take()
            {
                let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
            }
        }
    }

    /// Windows calls a low-level hook on the thread that installed it, and removes the hooks that
    /// answer late: it gets a thread of its own, which the callbacks of the host can't keep busy.
    fn start_key_hook() -> Option<u32> {
        let (started, thread) = mpsc::channel();
        std::thread::Builder::new()
            .name("rustino-kiosk-keys".into())
            .spawn(move || unsafe {
                let install = || SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), None, 0).ok();
                let hook = install();
                // The message queue of the thread exists before anyone posts `WM_QUIT` to it
                let mut message = MSG::default();
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                let _ = started.send(hook.is_some().then(|| GetCurrentThreadId()));
                let Some(mut hook) = hook else { return };
                // The hooks installed last get the keys first, and may keep them from the others;
                // Windows also removes, without telling, a hook that once answered late. Installed
                // again every now and then, the hook stays there and ahead of the others.
                let timer = SetTimer(None, 0, REINSTALL_MS, None);
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    if message.message == WM_TIMER
                        && let Some(new_hook) = install()
                    {
                        let _ = UnhookWindowsHookEx(hook);
                        hook = new_hook;
                    }
                }
                let _ = KillTimer(None, timer);
                let _ = UnhookWindowsHookEx(hook);
            })
            .ok()?;
        thread.recv().ok().flatten()
    }

    unsafe extern "system" fn key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let alt = key.flags.0 & LLKHF_ALTDOWN.0 != 0;
            let ctrl = unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) } < 0;
            let shift = unsafe { GetAsyncKeyState(i32::from(VK_SHIFT.0)) } < 0;
            if super::is_system_shortcut(key.vkCode, alt, ctrl, shift) {
                return LRESULT(1);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::cell::Cell;
    use std::sync::Once;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{msg_send, sel};
    use tao::platform::macos::WindowExtMacOS;
    use tao::window::{Fullscreen, Window};

    // NSApplicationPresentationOptions
    const HIDE_DOCK: usize = 1 << 1;
    const HIDE_MENU_BAR: usize = 1 << 3;
    const DISABLE_APPLE_MENU: usize = 1 << 4;
    const DISABLE_PROCESS_SWITCHING: usize = 1 << 5;
    const DISABLE_FORCE_QUIT: usize = 1 << 6;
    const DISABLE_SESSION_TERMINATION: usize = 1 << 7;
    const DISABLE_HIDE_APPLICATION: usize = 1 << 8;
    /// What AppKit has for kiosks: no Dock, menu bar, Cmd+Tab, Mission Control, Force Quit,
    /// Cmd+H, nor logout, restart and shutdown. They apply while the app is active.
    const KIOSK: usize = HIDE_DOCK
        | HIDE_MENU_BAR
        | DISABLE_APPLE_MENU
        | DISABLE_PROCESS_SWITCHING
        | DISABLE_FORCE_QUIT
        | DISABLE_SESSION_TERMINATION
        | DISABLE_HIDE_APPLICATION;

    /// The windows in kiosk mode: the presentation options are of the app
    static WINDOWS: AtomicUsize = AtomicUsize::new(0);

    thread_local! {
        /// The presentation options before the first window entered the kiosk mode
        static OPTIONS_BEFORE: Cell<usize> = const { Cell::new(0) };
    }

    pub struct Lock;

    impl Lock {
        pub fn new(window: &Window, _without_touchpad_gestures: bool) -> Self {
            if WINDOWS.fetch_add(1, Ordering::AcqRel) == 0 {
                OPTIONS_BEFORE.set(unsafe { msg_send![app(), presentationOptions] });
                keep_from_terminating();
            }
            // The presentation options of the kiosk aren't for the fullscreen of macOS, in a
            // space of its own: `hold` covers the screen once the window left it
            if window.fullscreen().is_some() {
                window.set_fullscreen(None);
            }
            Self
        }

        pub fn hold(&mut self, window: &Window) {
            if !window.simple_fullscreen()
                && (window.fullscreen().is_some() || !window.set_simple_fullscreen(true))
            {
                return;
            }
            unsafe {
                let app = app();
                let options: usize = msg_send![app, presentationOptions];
                if options != KIOSK {
                    let _: () = msg_send![app, setPresentationOptions: KIOSK];
                }
                // The presentation options turn off the Hide of the menus, not the one of other apps
                let hidden: bool = msg_send![app, isHidden];
                if hidden {
                    let _: () = msg_send![app, unhide: std::ptr::null::<AnyObject>()];
                }
                let active: bool = msg_send![app, isActive];
                if hidden || !active {
                    let _: () = msg_send![app, activateIgnoringOtherApps: true];
                    window.set_focus();
                }
            }
        }

        pub fn release(&mut self, window: &Window, fullscreen: bool) {
            // tao puts back the presentation options the window found, which are of the app:
            // they stay while another window is in kiosk mode
            window.set_simple_fullscreen(false);
            if WINDOWS.load(Ordering::Acquire) > 1 {
                let _: () = unsafe { msg_send![app(), setPresentationOptions: KIOSK] };
            }
            if fullscreen {
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }
        }
    }

    impl Drop for Lock {
        fn drop(&mut self) {
            if WINDOWS.fetch_sub(1, Ordering::AcqRel) == 1 {
                let _: () = unsafe { msg_send![app(), setPresentationOptions: OPTIONS_BEFORE.get()] };
            }
        }
    }

    fn app() -> *mut AnyObject {
        let class = AnyClass::get(c"NSApplication").expect("AppKit is loaded");
        unsafe { msg_send![class, sharedApplication] }
    }

    /// Cmd+Q, the Quit of the menus and the Apple events that quit the app go through
    /// `applicationShouldTerminate:`, which tao's app delegate doesn't have: it gets one that
    /// refuses while a window is in kiosk mode.
    fn keep_from_terminating() {
        extern "C-unwind" fn should_terminate(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> usize {
            // NSTerminateCancel, NSTerminateNow
            usize::from(WINDOWS.load(Ordering::Acquire) == 0)
        }
        static ADDED: Once = Once::new();
        ADDED.call_once(|| unsafe {
            let delegate: *mut AnyObject = msg_send![app(), delegate];
            if delegate.is_null() {
                return;
            }
            let class = objc2::ffi::object_getClass(delegate) as *mut AnyClass;
            type ShouldTerminate = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize;
            // No effect on a delegate of the host with its own answer
            objc2::ffi::class_addMethod(
                class,
                sel!(applicationShouldTerminate:),
                std::mem::transmute::<ShouldTerminate, Imp>(should_terminate),
                c"Q@:@".as_ptr(),
            );
        });
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use gtk::gdk;
    use gtk::prelude::*;
    use tao::platform::unix::WindowExtUnix;
    use tao::window::{Fullscreen, Window};

    pub struct Lock;

    impl Lock {
        pub fn new(_window: &Window, _without_touchpad_gestures: bool) -> Self {
            Self
        }

        pub fn hold(&mut self, window: &Window) {
            // The state of the window, not tao's `fullscreen`, which is what was asked last
            let fullscreen = window
                .gtk_window()
                .window()
                .is_some_and(|gdk_window| gdk_window.state().contains(gdk::WindowState::FULLSCREEN));
            if !fullscreen {
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }
            // A dialog of the app keeps the focus
            let app_is_active = gtk::Window::list_toplevels()
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Window>())
                .any(|toplevel| toplevel.is_active());
            if !app_is_active {
                window.set_focus();
            } else if window.gtk_window().is_active() {
                grab_keyboard(window);
            }
        }

        pub fn release(&mut self, window: &Window, fullscreen: bool) {
            ungrab_keyboard();
            if !fullscreen {
                window.set_fullscreen(None);
            }
        }
    }

    impl Drop for Lock {
        fn drop(&mut self) {
            ungrab_keyboard();
        }
    }

    /// With the keyboard grabbed, the window manager of X11 doesn't get its shortcuts (Super,
    /// Alt+Tab, Alt+F4, ...); on Wayland GDK asks the compositor to inhibit them, which may ask
    /// the user first. The keys still reach the windows of the app.
    fn grab_keyboard(window: &Window) {
        let Some(gdk_window) = window.gtk_window().window() else {
            return;
        };
        let display = gdk_window.display();
        let Some(seat) = display.default_seat() else { return };
        // Already grabbed, or by a menu of the app
        if seat.keyboard().is_some_and(|keyboard| display.device_is_grabbed(&keyboard)) {
            return;
        }
        let _ = seat.grab(&gdk_window, gdk::SeatCapabilities::KEYBOARD, true, None, None, None);
    }

    fn ungrab_keyboard() {
        if let Some(seat) = gdk::Display::default().and_then(|display| display.default_seat()) {
            seat.ungrab();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_system_shortcut;

    #[test]
    fn system_shortcuts() {
        const TAB: u32 = 0x09;
        const ESCAPE: u32 = 0x1B;
        const F4: u32 = 0x73;
        // The Windows keys, alone and with any other key
        assert!(is_system_shortcut(0x5B, false, false, false));
        assert!(is_system_shortcut(0x5C, false, false, false));
        assert!(is_system_shortcut(0xAA, false, false, false), "Dedicated Search key (Fn+F9 on Dell XPS)");
        assert!(is_system_shortcut(TAB, true, false, false), "Alt+Tab");
        assert!(is_system_shortcut(TAB, true, true, false), "Ctrl+Alt+Tab");
        assert!(is_system_shortcut(F4, true, false, false), "Alt+F4");
        assert!(is_system_shortcut(0x20, true, false, false), "Alt+Space");
        assert!(is_system_shortcut(ESCAPE, true, false, false), "Alt+Esc");
        assert!(is_system_shortcut(ESCAPE, false, true, false), "Ctrl+Esc, Ctrl+Shift+Esc");
        for key in [0x2C, 0xAC, 0xB4, 0xB5, 0xB6, 0xB7, 0x5F] {
            assert!(is_system_shortcut(key, false, false, false), "System key {key:#X}");
        }
        assert!(is_system_shortcut(0x2C, true, false, true), "Alt+Shift+PrintScreen");
        assert!(is_system_shortcut(0x90, true, false, true), "Alt+Shift+NumLock");
    }

    #[test]
    fn keys_of_the_page() {
        assert!(!is_system_shortcut(0x09, false, false, false), "Tab");
        assert!(!is_system_shortcut(0x09, false, true, false), "Ctrl+Tab");
        assert!(!is_system_shortcut(0x1B, false, false, false), "Esc");
        assert!(!is_system_shortcut(0x73, false, false, false), "F4");
        assert!(!is_system_shortcut(0x78, false, false, false), "F9: the function key remains available to the page");
        assert!(!is_system_shortcut(0x20, false, false, false), "Space");
        assert!(!is_system_shortcut(0x41, true, true, false), "Ctrl+Alt+A: a secret combination of the host");
        assert!(!is_system_shortcut(0x45, false, true, true), "Ctrl+Shift+E: the sample's kiosk exit");
        assert!(!is_system_shortcut(0x7B, false, false, false), "F12: developer tools only if the host enabled them");
        assert!(!is_system_shortcut(0x49, false, true, true), "Ctrl+Shift+I: as F12");
        assert!(!is_system_shortcut(0x90, false, false, false), "NumLock");
        for key in [0x10, 0xA0, 0xA1] {
            assert!(!is_system_shortcut(key, false, false, true), "Shift stays available to the page");
        }
    }
}
