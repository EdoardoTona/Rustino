//! Temporarily disables Windows accessibility activation shortcuts while a kiosk window runs.
//! The features themselves remain usable. Changes affect this logon session, without updating
//! the user profile. A volatile backup scoped to this logon lets a later kiosk restore the
//! original shortcuts after a forced process exit.

use std::sync::Mutex;

use windows::Win32::Foundation::{CloseHandle, E_FAIL, ERROR_FILE_NOT_FOUND, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_STATISTICS, TokenStatistics};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_OPTION_VOLATILE, RRF_RT_REG_DWORD,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Accessibility::{FILTERKEYS, HCF_OPTION_NOTHEMECHANGE, HIGHCONTRASTW, MOUSEKEYS, STICKYKEYS};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETFILTERKEYS, SPI_GETHIGHCONTRAST, SPI_GETMOUSEKEYS, SPI_GETSTICKYKEYS,
    SPI_SETFILTERKEYS, SPI_SETHIGHCONTRAST, SPI_SETMOUSEKEYS, SPI_SETSTICKYKEYS,
    SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{Error, HSTRING, PCWSTR};

// HOTKEYACTIVE and CONFIRMHOTKEY have the same values in all four structures. Preserve
// every other flag, including FEATUREON, so existing accessibility features keep working.
const SHORTCUT_FLAGS: u32 = 0x04 | 0x08;

#[derive(Clone, Copy, Debug)]
enum Setting {
    StickyKeys,
    FilterKeys,
    MouseKeys,
    HighContrast,
}

impl Setting {
    const ALL: [Self; 4] = [Self::StickyKeys, Self::FilterKeys, Self::MouseKeys, Self::HighContrast];

    fn name(self) -> &'static str {
        match self {
            Self::StickyKeys => "StickyKeys",
            Self::FilterKeys => "FilterKeys",
            Self::MouseKeys => "MouseKeys",
            Self::HighContrast => "HighContrast",
        }
    }

    fn update(self, replace: Option<u32>) -> windows::core::Result<u32> {
        match self {
            Self::StickyKeys => update(
                STICKYKEYS { cbSize: size_of::<STICKYKEYS>() as u32, ..Default::default() },
                SPI_GETSTICKYKEYS, SPI_SETSTICKYKEYS, |p| &mut p.dwFlags.0, replace,
            ),
            Self::FilterKeys => update(
                FILTERKEYS { cbSize: size_of::<FILTERKEYS>() as u32, ..Default::default() },
                SPI_GETFILTERKEYS, SPI_SETFILTERKEYS, |p| &mut p.dwFlags, replace,
            ),
            Self::MouseKeys => update(
                MOUSEKEYS { cbSize: size_of::<MOUSEKEYS>() as u32, ..Default::default() },
                SPI_GETMOUSEKEYS, SPI_SETMOUSEKEYS, |p| &mut p.dwFlags, replace,
            ),
            Self::HighContrast => update(
                Contrast(HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() }),
                SPI_GETHIGHCONTRAST, SPI_SETHIGHCONTRAST,
                |p| {
                    // Changing only a shortcut must not reload the desktop theme.
                    p.0.dwFlags |= HCF_OPTION_NOTHEMECHANGE;
                    &mut p.0.dwFlags.0
                },
                replace,
            ),
        }
    }
}

#[repr(transparent)]
struct Contrast(HIGHCONTRASTW);

impl Drop for Contrast {
    fn drop(&mut self) {
        // SPI_GETHIGHCONTRAST allocates the scheme name; it is not part of our snapshot.
        if !self.0.lpszDefaultScheme.is_null() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0.lpszDefaultScheme.0.cast()))) };
        }
    }
}

fn update<T>(
    mut parameters: T,
    get: SYSTEM_PARAMETERS_INFO_ACTION,
    set: SYSTEM_PARAMETERS_INFO_ACTION,
    flags: impl FnOnce(&mut T) -> &mut u32,
    replace: Option<u32>,
) -> windows::core::Result<u32> {
    let size = size_of::<T>() as u32;
    // No SPIF_UPDATEINIFILE: keep the user's persisted preferences untouched.
    let session_only = SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0);
    unsafe { SystemParametersInfoW(get, size, Some((&mut parameters as *mut T).cast()), session_only)? };
    let flags = flags(&mut parameters);
    let original = *flags & SHORTCUT_FLAGS;
    let Some(replace) = replace else { return Ok(original) };
    let desired = (*flags & !SHORTCUT_FLAGS) | (replace & SHORTCUT_FLAGS);
    let changed = *flags != desired;
    *flags = desired;
    if changed {
        unsafe { SystemParametersInfoW(set, size, Some((&mut parameters as *mut T).cast()), session_only)? };
    }
    Ok(original)
}

struct State {
    windows: usize,
    saved: Vec<(Setting, u32)>,
    recovery_path: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State { windows: 0, saved: Vec::new(), recovery_path: None });

/// Separate interactive logons can share a user hive but have different active settings.
fn recovery_path() -> windows::core::Result<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    let mut statistics = TOKEN_STATISTICS::default();
    let mut length = 0;
    let result = unsafe {
        GetTokenInformation(token, TokenStatistics, Some((&mut statistics as *mut TOKEN_STATISTICS).cast()),
            size_of::<TOKEN_STATISTICS>() as u32, &mut length)
    };
    let _ = unsafe { CloseHandle(token) };
    result?;
    let logon = statistics.AuthenticationId;
    Ok(format!(r"Software\Rustino\Kiosk\AccessibilityShortcuts\{:08X}{:08X}", logon.HighPart as u32, logon.LowPart))
}

struct RecoveryKey(HKEY);

impl RecoveryKey {
    fn open(path: &str) -> windows::core::Result<Self> {
        let (parent_path, name) = path.rsplit_once('\\').ok_or_else(|| Error::from(E_FAIL))?;
        let mut parent = HKEY::default();
        unsafe {
            // Keep our shared application parents nonvolatile: the optional touchpad backup
            // also lives below Software\Rustino\Kiosk and must survive a system restart.
            RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(parent_path), None, PCWSTR::null(),
                REG_OPTION_NON_VOLATILE, KEY_READ | KEY_WRITE, None, &mut parent, None).ok()?;
        }
        let parent = Self(parent);
        let mut key = HKEY::default();
        unsafe {
            RegCreateKeyExW(parent.0, &HSTRING::from(name), None, PCWSTR::null(),
                REG_OPTION_VOLATILE, KEY_READ | KEY_WRITE, None, &mut key, None).ok()?;
        }
        Ok(Self(key))
    }

    fn read(&self, name: &str) -> windows::core::Result<Option<u32>> {
        let (mut value, mut size) = (0u32, size_of::<u32>() as u32);
        let status = unsafe {
            RegGetValueW(self.0, PCWSTR::null(), &HSTRING::from(name), RRF_RT_REG_DWORD, None,
                Some((&mut value as *mut u32).cast()), Some(&mut size))
        };
        if status == ERROR_FILE_NOT_FOUND { return Ok(None) }
        status.ok()?;
        Ok(Some(value))
    }

    fn write(&self, name: &str, value: u32) -> windows::core::Result<()> {
        unsafe {
            RegSetKeyValueW(self.0, PCWSTR::null(), &HSTRING::from(name), REG_DWORD.0,
                Some((&value as *const u32).cast()), size_of::<u32>() as u32).ok()
        }
    }
}

impl Drop for RecoveryKey {
    fn drop(&mut self) { let _ = unsafe { RegCloseKey(self.0) }; }
}

fn snapshot() -> windows::core::Result<(String, Vec<(Setting, u32)>)> {
    let path = recovery_path()?;
    let key = RecoveryKey::open(&path)?;
    let mut saved = Vec::new();
    if key.read("Saved")? == Some(1) {
        // A previous process ended before it could restore its shortcuts.
        for setting in Setting::ALL {
            let value = key.read(setting.name())?.ok_or_else(|| Error::from(E_FAIL))?;
            saved.push((setting, value & SHORTCUT_FLAGS));
        }
    } else {
        for setting in Setting::ALL {
            let original = setting.update(None)?;
            key.write(setting.name(), original)?;
            saved.push((setting, original));
        }
        // Commit the complete recovery snapshot before changing any system setting.
        key.write("Saved", 1)?;
    }
    Ok((path, saved))
}

pub struct Lock(());

impl Lock {
    pub fn new() -> Self {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if state.windows == 0 {
            match snapshot() {
                Ok((path, saved)) => {
                    state.recovery_path = Some(path);
                    state.saved = saved;
                    for &(setting, _) in &state.saved {
                        if let Err(error) = setting.update(Some(0)) {
                            eprintln!("Rustino kiosk: could not disable {setting:?} shortcuts: {error}");
                        }
                    }
                }
                Err(error) => eprintln!("Rustino kiosk: could not save accessibility shortcuts; settings left unchanged: {error}"),
            }
        }
        state.windows += 1;
        Self(())
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.windows -= 1;
        if state.windows == 0 {
            let mut restored = true;
            for (setting, original) in state.saved.drain(..) {
                // Read the current structure again: only restore the shortcut bits, keeping
                // any unrelated accessibility settings changed while the kiosk was running.
                if let Err(error) = setting.update(Some(original)) {
                    restored = false;
                    eprintln!("Rustino kiosk: could not restore {setting:?} shortcuts: {error}");
                }
            }
            if let Some(path) = state.recovery_path.take().filter(|_| restored) {
                let cleanup = || -> windows::core::Result<()> {
                    // Invalidate a restored snapshot even if removing its key fails.
                    RecoveryKey::open(&path)?.write("Saved", 0)?;
                    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path)).ok() }
                };
                if let Err(error) = cleanup() {
                    eprintln!("Rustino kiosk: could not remove the accessibility recovery snapshot: {error}");
                }
            }
        }
    }
}
