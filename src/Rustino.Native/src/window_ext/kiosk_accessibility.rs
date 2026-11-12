//! Temporarily disables Windows accessibility activation shortcuts while a kiosk window runs.
//! The features themselves remain usable. Changes affect this logon session, without updating
//! the user profile. A volatile backup scoped to this logon lets a later kiosk restore the
//! original shortcuts after a forced process exit.
//!
//! The settings belong to the logon, not to the process: the processes in kiosk mode share the
//! backup, and the last one that leaves restores it.

use std::sync::Mutex;

use windows::Win32::Foundation::{CloseHandle, E_FAIL, ERROR_FILE_NOT_FOUND, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_STATISTICS, TokenStatistics};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_OPTION_VOLATILE, RRF_RT_REG_DWORD,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, INFINITE, OpenEventW, OpenProcessToken, ReleaseMutex,
    SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
};
use windows::Win32::UI::Accessibility::{
    FILTERKEYS, HCF_OPTION_NOTHEMECHANGE, HIGHCONTRASTW, MOUSEKEYS, STICKYKEYS, TOGGLEKEYS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETFILTERKEYS, SPI_GETHIGHCONTRAST, SPI_GETMOUSEKEYS, SPI_GETSTICKYKEYS, SPI_GETTOGGLEKEYS,
    SPI_SETFILTERKEYS, SPI_SETHIGHCONTRAST, SPI_SETMOUSEKEYS, SPI_SETSTICKYKEYS, SPI_SETTOGGLEKEYS,
    SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{Error, HSTRING, PCWSTR};

// HOTKEYACTIVE and CONFIRMHOTKEY have the same values in all five structures. Preserve
// every other flag, including FEATUREON, so existing accessibility features keep working.
const SHORTCUT_FLAGS: u32 = 0x04 | 0x08;

#[derive(Clone, Copy, Debug)]
enum Setting {
    StickyKeys,
    FilterKeys,
    /// NumLock held for five seconds
    ToggleKeys,
    MouseKeys,
    HighContrast,
}

impl Setting {
    const ALL: [Self; 5] = [
        Self::StickyKeys,
        Self::FilterKeys,
        Self::ToggleKeys,
        Self::MouseKeys,
        Self::HighContrast,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::StickyKeys => "StickyKeys",
            Self::FilterKeys => "FilterKeys",
            Self::ToggleKeys => "ToggleKeys",
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
            Self::ToggleKeys => update(
                TOGGLEKEYS { cbSize: size_of::<TOGGLEKEYS>() as u32, ..Default::default() },
                SPI_GETTOGGLEKEYS, SPI_SETTOGGLEKEYS, |p| &mut p.dwFlags, replace,
            ),
            Self::MouseKeys => update(
                MOUSEKEYS { cbSize: size_of::<MOUSEKEYS>() as u32, ..Default::default() },
                SPI_GETMOUSEKEYS, SPI_SETMOUSEKEYS, |p| &mut p.dwFlags, replace,
            ),
            // The scheme name the system fills in is passed back as it came and never freed.
            // The documentation of HIGHCONTRASTW says to free it with LocalFree, but user32
            // keeps the buffer and hands out the same one at every call: on Windows 11 26200
            // the call after a LocalFree ends the process with STATUS_HEAP_CORRUPTION.
            Self::HighContrast => update(
                HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() },
                SPI_GETHIGHCONTRAST, SPI_SETHIGHCONTRAST,
                |p| {
                    // Changing only a shortcut must not reload the desktop theme.
                    p.dwFlags |= HCF_OPTION_NOTHEMECHANGE;
                    &mut p.dwFlags.0
                },
                replace,
            ),
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

/// Separate interactive logons can share a user hive but have different active settings.
fn logon_id() -> windows::core::Result<String> {
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
    Ok(format!("{:08X}{:08X}", logon.HighPart as u32, logon.LowPart))
}

fn recovery_path(logon: &str) -> String {
    format!(r"Software\Rustino\Kiosk\AccessibilityShortcuts\{logon}")
}

/// Keeps the other processes of this logon out while one takes or restores the snapshot.
struct Exclusive(HANDLE);

impl Exclusive {
    fn acquire(logon: &str) -> windows::core::Result<Self> {
        let name = HSTRING::from(format!(r"Local\Rustino.Kiosk.AccessibilityShortcuts.{logon}.Lock"));
        let mutex = unsafe { CreateMutexW(None, false, &name)? };
        // Abandoned by a process that ended while holding it: it's ours all the same.
        unsafe { WaitForSingleObject(mutex, INFINITE) };
        Ok(Self(mutex))
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// A named object that lives as long as a process of this logon has shortcuts disabled: the
/// system closes the handle of a process that ends, however it ends.
struct InUse(HANDLE);

// A kernel handle, usable from any thread.
unsafe impl Send for InUse {}

impl InUse {
    fn name(logon: &str) -> HSTRING {
        HSTRING::from(format!(r"Local\Rustino.Kiosk.AccessibilityShortcuts.{logon}.InUse"))
    }

    fn join(logon: &str) -> windows::core::Result<Self> {
        Ok(Self(unsafe { CreateEventW(None, true, false, &Self::name(logon))? }))
    }

    /// Whether another process still holds it. Call it after dropping ours.
    fn held(logon: &str) -> bool {
        match unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &Self::name(logon)) } {
            Ok(other) => {
                let _ = unsafe { CloseHandle(other) };
                true
            }
            Err(_) => false,
        }
    }
}

impl Drop for InUse {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
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

/// Saves the shortcuts to put back, or brings up to date the ones already saved.
fn snapshot(logon: &str) -> windows::core::Result<()> {
    let key = RecoveryKey::open(&recovery_path(logon))?;
    // Saved already: another process is in kiosk mode, or one ended before it could restore
    // its shortcuts.
    let saved = key.read("Saved")? == Some(1);
    for setting in Setting::ALL {
        let current = setting.update(None)?;
        // A setting the snapshot doesn't have was never turned off: an older version left it.
        let stored = if saved { key.read(setting.name())? } else { None };
        // The kiosk left the saved ones off: one that is on was set by the user meanwhile,
        // and that choice replaces the one saved before.
        let original = match stored {
            Some(stored) if current == 0 => stored & SHORTCUT_FLAGS,
            _ => current,
        };
        if stored != Some(original) {
            key.write(setting.name(), original)?;
        }
    }
    // Commit the complete recovery snapshot before changing any system setting.
    if !saved {
        key.write("Saved", 1)?;
    }
    Ok(())
}

/// The shortcuts to put back: the snapshot is shared, and a process that came later may
/// have brought it up to date.
fn saved(logon: &str) -> windows::core::Result<Vec<(Setting, u32)>> {
    let key = RecoveryKey::open(&recovery_path(logon))?;
    if key.read("Saved")? != Some(1) {
        return Err(Error::from(E_FAIL));
    }
    Setting::ALL
        .into_iter()
        .map(|setting| {
            let stored = key.read(setting.name())?.ok_or_else(|| Error::from(E_FAIL))?;
            Ok((setting, stored & SHORTCUT_FLAGS))
        })
        .collect()
}

/// The shortcuts this process turned off.
struct Session {
    logon: String,
    in_use: InUse,
}

impl Session {
    fn begin() -> windows::core::Result<Self> {
        let logon = logon_id()?;
        let _exclusive = Exclusive::acquire(&logon)?;
        let in_use = InUse::join(&logon)?;
        snapshot(&logon)?;
        for setting in Setting::ALL {
            if let Err(error) = setting.update(Some(0)) {
                eprintln!("Rustino kiosk: could not disable {setting:?} shortcuts: {error}");
            }
        }
        Ok(Self { logon, in_use })
    }

    fn end(self) {
        let Self { logon, in_use } = self;
        let _exclusive = Exclusive::acquire(&logon);
        drop(in_use);
        if InUse::held(&logon) {
            // Another process is still in kiosk mode: the last one restores the shortcuts.
            return;
        }
        let saved = match saved(&logon) {
            Ok(saved) => saved,
            Err(error) => {
                eprintln!("Rustino kiosk: could not read the saved accessibility shortcuts: {error}");
                return;
            }
        };
        let mut restored = true;
        for (setting, original) in saved {
            // Read the current structure again: only restore the shortcut bits, keeping
            // any unrelated accessibility settings changed while the kiosk was running.
            if let Err(error) = setting.update(Some(original)) {
                restored = false;
                eprintln!("Rustino kiosk: could not restore {setting:?} shortcuts: {error}");
            }
        }
        if restored {
            let path = recovery_path(&logon);
            let cleanup = || -> windows::core::Result<()> {
                // Invalidate a restored snapshot even if removing its key fails.
                RecoveryKey::open(&path)?.write("Saved", 0)?;
                unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path.as_str())).ok() }
            };
            if let Err(error) = cleanup() {
                eprintln!("Rustino kiosk: could not remove the accessibility recovery snapshot: {error}");
            }
        }
    }
}

struct State {
    windows: usize,
    session: Option<Session>,
}

static STATE: Mutex<State> = Mutex::new(State { windows: 0, session: None });

pub struct Lock(());

impl Lock {
    pub fn new() -> Self {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if state.windows == 0 {
            match Session::begin() {
                Ok(session) => state.session = Some(session),
                Err(error) => {
                    eprintln!("Rustino kiosk: could not save accessibility shortcuts; settings left unchanged: {error}")
                }
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
        if state.windows == 0
            && let Some(session) = state.session.take()
        {
            session.end();
        }
    }
}
