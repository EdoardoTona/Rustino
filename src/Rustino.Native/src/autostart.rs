//! Start at login for the current user: a value of the Run key on Windows, a launch agent in
//! `~/Library/LaunchAgents` on macOS, an XDG autostart entry on Linux. Each one also tells when
//! the user turned the entry off from the system: Task Manager or Settings › Apps › Startup on
//! Windows, System Settings › Login Items on macOS, the startup applications of GNOME and KDE.

use std::ffi::c_char;
use std::panic::catch_unwind;

// Values of the host API (AutoStartStatus)
#[cfg_attr(target_os = "windows", allow(dead_code))]
const UNSUPPORTED: i32 = 0;
const DISABLED: i32 = 1;
const ENABLED: i32 = 2;
/// The entry is there, but the user turned it off from the system
const DISABLED_BY_SYSTEM: i32 = 3;
const FAILED: i32 = -1;

/// Status of the entry of `app_id`; -1 for an invalid id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_autostart_status(app_id: *const c_char) -> i32 {
    catch_unwind(|| match unsafe { app_id_arg(app_id) } {
        Some(app_id) => platform::status(&app_id),
        None => FAILED,
    })
    .unwrap_or(FAILED)
}

/// Writes the entry of `app_id`, which runs `command` (the program and its arguments, separated
/// by '\n') at login, and returns the status that follows; -1 when the entry can't be written.
/// On Windows it also clears the choice of Task Manager; macOS keeps the one of Login Items.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_autostart_enable(
    app_id: *const c_char,
    display_name: *const c_char,
    command: *const c_char,
) -> i32 {
    catch_unwind(|| {
        let Some(app_id) = (unsafe { app_id_arg(app_id) }) else {
            return FAILED;
        };
        let command: Vec<String> = unsafe { crate::util::cstr_to_string(command) }
            .map(|command| command.split('\n').map(str::to_owned).collect())
            .unwrap_or_default();
        if command.first().is_none_or(String::is_empty) {
            return FAILED;
        }
        let display_name = unsafe { crate::util::cstr_to_string(display_name) }
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| app_id.clone());
        match platform::enable(&app_id, &display_name, &command) {
            Ok(()) => platform::status(&app_id),
            Err(_) => FAILED,
        }
    })
    .unwrap_or(FAILED)
}

/// Removes the entry of `app_id` and returns the status that follows; -1 when it can't.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_autostart_disable(app_id: *const c_char) -> i32 {
    catch_unwind(|| {
        let Some(app_id) = (unsafe { app_id_arg(app_id) }) else {
            return FAILED;
        };
        match platform::disable(&app_id) {
            Ok(()) => platform::status(&app_id),
            Err(_) => FAILED,
        }
    })
    .unwrap_or(FAILED)
}

/// Opens the system page where the user turns the startup apps on and off: 0 on Linux, which
/// has none in common.
#[unsafe(no_mangle)]
pub extern "C" fn rustino_autostart_open_settings() -> i32 {
    catch_unwind(|| i32::from(platform::open_settings())).unwrap_or(0)
}

unsafe fn app_id_arg(app_id: *const c_char) -> Option<String> {
    unsafe { crate::util::cstr_to_string(app_id) }.filter(|id| is_valid_app_id(id))
}

/// The id names a registry value, a launchd label and a file: letters, digits, '.', '-' and '_'.
pub(crate) fn is_valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// A command line that `CommandLineToArgvW` and the C runtime split back into `args`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_command_line(args: &[String]) -> String {
    let mut line = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
            line.push_str(arg);
            continue;
        }
        line.push('"');
        let mut backslashes = 0;
        for c in arg.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    // The backslashes before a quote and the quote itself are escaped
                    line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    line.push('"');
                    backslashes = 0;
                }
                _ => {
                    line.extend(std::iter::repeat_n('\\', backslashes));
                    line.push(c);
                    backslashes = 0;
                }
            }
        }
        // Before the closing quote
        line.extend(std::iter::repeat_n('\\', backslashes * 2));
        line.push('"');
    }
    line
}

/// A string value of a desktop entry (escapes of the Desktop Entry Specification).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// The `Exec` key of a desktop entry: the arguments with reserved characters are quoted, then
/// the whole value is escaped as a string, and '%' doubled so it isn't a field code.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_exec(args: &[String]) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`',
    ];
    let quoted: Vec<String> = args
        .iter()
        .map(|arg| {
            if !arg.is_empty() && !arg.contains(RESERVED) {
                return arg.clone();
            }
            let mut out = String::from('"');
            for c in arg.chars() {
                if matches!(c, '"' | '`' | '$' | '\\') {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push('"');
            out
        })
        .collect();
    desktop_string(&quoted.join(" ")).replace('%', "%%")
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(display_name: &str, args: &[String]) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        desktop_string(display_name),
        desktop_exec(args)
    )
}

/// Whether the user turned the entry off: GNOME sets `X-GNOME-Autostart-enabled=false`, KDE
/// `Hidden=true`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry_disabled(content: &str) -> bool {
    let mut in_entry = false;
    let mut disabled = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry && let Some((key, value)) = line.split_once('=') {
            match (key.trim(), value.trim()) {
                ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false") => disabled = true,
                _ => {}
            }
        }
    }
    disabled
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_text(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The launch agent: the app starts once at login, in the graphical session.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launch_agent_plist(app_id: &str, args: &[String], bundle_id: Option<&str>) -> String {
    let arguments: String = args
        .iter()
        .map(|arg| format!("\t\t<string>{}</string>\n", xml_text(arg)))
        .collect();
    // Login Items shows the agent under the app's name and icon
    let bundle = bundle_id
        .map(|id| format!("\t<key>AssociatedBundleIdentifiers</key>\n\t<string>{}</string>\n", xml_text(id)))
        .unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{arguments}\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>LimitLoadToSessionType</key>\n\t<string>Aqua</string>\n\
         \t<key>ProcessType</key>\n\t<string>Interactive</string>\n\
         {bundle}</dict>\n</plist>\n",
        xml_text(app_id)
    )
}

/// Writes through a temporary file, so a reader never sees half an entry.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_file(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, content)?;
    std::fs::rename(&temp, path)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn remove_file(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;
    use std::io;

    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW,
        RegGetValueW, RegSetKeyValueW,
    };
    use windows::core::HSTRING;

    use super::{DISABLED, DISABLED_BY_SYSTEM, ENABLED};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    /// Task Manager and Settings turn the entries of Run off here, without removing them: the
    /// first byte of the value is odd (03) when off, even (02) when on
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    pub fn status(app_id: &str) -> i32 {
        let name = HSTRING::from(app_id);
        let mut size = 0u32;
        let run = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN), &name, RRF_RT_REG_SZ, None, None, Some(&mut size))
        };
        if run.is_err() {
            return DISABLED;
        }
        let mut data = [0u8; 12];
        let mut size = data.len() as u32;
        let approved = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(APPROVED),
                &name,
                RRF_RT_REG_BINARY,
                None,
                Some(data.as_mut_ptr() as *mut c_void),
                Some(&mut size),
            )
        };
        if approved.is_ok() && size > 0 && data[0] & 1 == 1 { DISABLED_BY_SYSTEM } else { ENABLED }
    }

    pub fn enable(app_id: &str, _display_name: &str, command: &[String]) -> io::Result<()> {
        let data: Vec<u8> = super::windows_command_line(command)
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        let name = HSTRING::from(app_id);
        check(unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN),
                &name,
                REG_SZ.0,
                Some(data.as_ptr() as *const c_void),
                data.len() as u32,
            )
        })?;
        // On again: the entry starts as the ones Task Manager never touched
        delete(APPROVED, app_id)
    }

    pub fn disable(app_id: &str) -> io::Result<()> {
        delete(RUN, app_id)?;
        delete(APPROVED, app_id)
    }

    pub fn open_settings() -> bool {
        crate::external::open_external("ms-settings:startupapps")
    }

    fn delete(key: &str, app_id: &str) -> io::Result<()> {
        let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(app_id)) };
        if result == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(result) }
    }

    fn check(result: WIN32_ERROR) -> io::Result<()> {
        if result.is_ok() { Ok(()) } else { Err(io::Error::from_raw_os_error(result.0 as i32)) }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::io;
    use std::path::PathBuf;

    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyClass;
    use objc2_foundation::{NSBundle, NSString, NSURL};

    use super::{DISABLED, DISABLED_BY_SYSTEM, ENABLED, UNSUPPORTED};

    // SMAppService (macOS 13+) tells when the user turned the agent off in Login Items
    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    /// SMAppServiceStatusRequiresApproval
    const REQUIRES_APPROVAL: isize = 2;

    fn plist_path(app_id: &str) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
        Some(PathBuf::from(home).join("Library/LaunchAgents").join(format!("{app_id}.plist")))
    }

    pub fn status(app_id: &str) -> i32 {
        let Some(path) = plist_path(app_id) else {
            return UNSUPPORTED;
        };
        if !path.exists() {
            return DISABLED;
        }
        if legacy_status(&path) == Some(REQUIRES_APPROVAL) { DISABLED_BY_SYSTEM } else { ENABLED }
    }

    /// `None` before macOS 13, which has no switch for launch agents
    fn legacy_status(path: &std::path::Path) -> Option<isize> {
        let class = AnyClass::get(c"SMAppService")?;
        let path = path.to_str()?;
        autoreleasepool(|_| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            Some(unsafe { objc2::msg_send![class, statusForLegacyURL: &*url] })
        })
    }

    pub fn enable(app_id: &str, _display_name: &str, command: &[String]) -> io::Result<()> {
        let path = plist_path(app_id).ok_or(io::ErrorKind::NotFound)?;
        let bundle_id = autoreleasepool(|_| NSBundle::mainBundle().bundleIdentifier().map(|id| id.to_string()));
        super::write_file(&path, &super::launch_agent_plist(app_id, command, bundle_id.as_deref()))
    }

    /// The agent loaded at login stays loaded until logout: unloading it would quit the app
    /// it started. Without the plist it doesn't load again.
    pub fn disable(app_id: &str) -> io::Result<()> {
        match plist_path(app_id) {
            Some(path) => super::remove_file(&path),
            None => Ok(()),
        }
    }

    pub fn open_settings() -> bool {
        match AnyClass::get(c"SMAppService") {
            Some(class) => {
                let _: () = unsafe { objc2::msg_send![class, openSystemSettingsLoginItems] };
                true
            }
            None => crate::external::open_external("x-apple.systempreferences:com.apple.preferences.users"),
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::io;
    use std::path::PathBuf;

    use super::{DISABLED, DISABLED_BY_SYSTEM, ENABLED, UNSUPPORTED};

    /// `$XDG_CONFIG_HOME/autostart/<app_id>.desktop`. A Flatpak sandbox has its own config
    /// folder, which the session doesn't read: there the Background portal starts apps instead.
    fn entry_path(app_id: &str) -> Option<PathBuf> {
        if std::env::var_os("FLATPAK_ID").is_some() {
            return None;
        }
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
                Some(PathBuf::from(home).join(".config"))
            })?;
        Some(config.join("autostart").join(format!("{app_id}.desktop")))
    }

    pub fn status(app_id: &str) -> i32 {
        let Some(path) = entry_path(app_id) else {
            return UNSUPPORTED;
        };
        match std::fs::read_to_string(&path) {
            Ok(content) if super::desktop_entry_disabled(&content) => DISABLED_BY_SYSTEM,
            Ok(_) => ENABLED,
            Err(_) => DISABLED,
        }
    }

    pub fn enable(app_id: &str, display_name: &str, command: &[String]) -> io::Result<()> {
        let path = entry_path(app_id).ok_or(io::ErrorKind::Unsupported)?;
        super::write_file(&path, &super::desktop_entry(display_name, command))
    }

    pub fn disable(app_id: &str) -> io::Result<()> {
        match entry_path(app_id) {
            Some(path) => super::remove_file(&path),
            None => Ok(()),
        }
    }

    pub fn open_settings() -> bool {
        false
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    pub fn status(_app_id: &str) -> i32 {
        super::UNSUPPORTED
    }

    pub fn enable(_app_id: &str, _display_name: &str, _command: &[String]) -> std::io::Result<()> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    pub fn disable(_app_id: &str) -> std::io::Result<()> {
        Ok(())
    }

    pub fn open_settings() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|&v| v.to_owned()).collect()
    }

    #[test]
    fn app_ids() {
        assert!(is_valid_app_id("com.example.App"));
        assert!(is_valid_app_id("my-app_2"));
        assert!(!is_valid_app_id(""));
        assert!(!is_valid_app_id(".hidden"));
        assert!(!is_valid_app_id("a/b"));
        assert!(!is_valid_app_id("a b"));
        assert!(!is_valid_app_id(r"a\b"));
        assert!(!is_valid_app_id(&"a".repeat(129)));
    }

    #[test]
    fn windows_quoting() {
        assert_eq!(
            windows_command_line(&args(&[r"C:\Program Files\App\app.exe", "--autostart"])),
            r#""C:\Program Files\App\app.exe" --autostart"#
        );
        assert_eq!(windows_command_line(&args(&[r"C:\App\app.exe", ""])), r#"C:\App\app.exe """#);
        assert_eq!(windows_command_line(&args(&[r#"say "hi""#])), r#""say \"hi\"""#);
        assert_eq!(windows_command_line(&args(&[r"C:\dir with space\"])), r#""C:\dir with space\\""#);
        assert_eq!(windows_command_line(&args(&[r#"a\"b"#])), r#""a\\\"b""#);
        assert_eq!(windows_command_line(&args(&[r"a\b"])), r"a\b");
    }

    #[test]
    fn desktop_exec_quoting() {
        assert_eq!(desktop_exec(&args(&["/opt/app/app", "--autostart"])), "/opt/app/app --autostart");
        // Quoted, then the backslash escaped again as a string value
        assert_eq!(desktop_exec(&args(&["/home/me/My App/app"])), r#""/home/me/My App/app""#);
        assert_eq!(desktop_exec(&args(&["a$b"])), r#""a\\$b""#);
        assert_eq!(desktop_exec(&args(&["50%"])), "50%%");
        assert_eq!(desktop_string("a\nb\\c"), r"a\nb\\c");
    }

    #[test]
    fn desktop_entry_switches() {
        let entry = desktop_entry("My App", &args(&["/opt/app/app"]));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=My App\nExec=/opt/app/app\n"));
        assert!(!desktop_entry_disabled(&entry));
        assert!(desktop_entry_disabled(&entry.replace("X-GNOME-Autostart-enabled=true", "X-GNOME-Autostart-enabled=false")));
        assert!(desktop_entry_disabled(&format!("{entry}Hidden=true\n")));
        // Keys of other groups don't count
        assert!(!desktop_entry_disabled(&format!("{entry}[Desktop Action x]\nHidden=true\n")));
    }

    #[test]
    fn launch_agent() {
        let plist = launch_agent_plist("com.example.App", &args(&["/Apps/A&B.app/Contents/MacOS/A&B", "--autostart"]), Some("com.example.App"));
        assert!(plist.contains("<key>Label</key>\n\t<string>com.example.App</string>"));
        assert!(plist.contains("\t\t<string>/Apps/A&amp;B.app/Contents/MacOS/A&amp;B</string>\n\t\t<string>--autostart</string>\n"));
        assert!(plist.contains("<key>AssociatedBundleIdentifiers</key>"));
        assert!(!launch_agent_plist("a", &args(&["/a"]), None).contains("AssociatedBundleIdentifiers"));
    }
}
