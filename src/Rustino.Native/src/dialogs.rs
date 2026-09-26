//! Native dialogs, modal for the window: sheets on macOS, owned by the window on Windows,
//! transient for it on Linux. While the window runs they run on its event loop thread, where
//! the parent window lives; before (or after) that they have no parent.

use tao::window::Window;

use crate::window::RustinoWindow;

#[derive(Debug)]
pub struct DialogParams {
    pub title: Option<String>,
    pub default_path: Option<String>,
    pub filters: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, Copy)]
pub enum FileDialogKind {
    Open { multiple: bool },
    Save,
    Folder { multiple: bool },
}

/// Returns the chosen paths, `None` when canceled.
pub fn file_dialog(
    instance: Option<&RustinoWindow>,
    kind: FileDialogKind,
    params: DialogParams,
) -> Option<Vec<String>> {
    match instance {
        Some(inst) if inst.is_running() => inst
            .invoke(move |window, _| show_file_dialog(kind, &params, Some(window)))
            .flatten(),
        _ => show_detached_file_dialog(kind, params),
    }
}

/// Shows the dialog on the event loop thread, modal for `parent`.
pub fn show_file_dialog(kind: FileDialogKind, params: &DialogParams, parent: Option<&Window>) -> Option<Vec<String>> {
    let dialog = build_file_dialog(params);
    #[cfg(not(target_os = "linux"))]
    {
        let dialog = match parent {
            Some(window) => dialog.set_parent(window),
            None => dialog,
        };
        pick(dialog, kind)
    }
    // The portal shows the dialog in another process: waiting for it must not block GTK, or
    // the window stops redrawing and the desktop reports it as not responding
    #[cfg(target_os = "linux")]
    {
        let dialog = match parent.and_then(linux::DialogParent::of) {
            Some(parent) => dialog.set_parent(&parent),
            None => dialog,
        };
        linux::wait_without_blocking_gtk(move || pick(dialog, kind)).flatten()
    }
}

/// Without a running window: macOS shows the dialog on the main thread, the other platforms on
/// their own thread.
fn show_detached_file_dialog(kind: FileDialogKind, params: DialogParams) -> Option<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        // rfd would dispatch_sync to the main thread, which would wait for another thread here
        pick(build_file_dialog(&params), kind)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            #[cfg(target_os = "windows")]
            init_com();
            let _ = tx.send(pick(build_file_dialog(&params), kind));
        });
        rx.recv().ok().flatten()
    }
}

fn pick(dialog: rfd::FileDialog, kind: FileDialogKind) -> Option<Vec<String>> {
    let paths = match kind {
        FileDialogKind::Open { multiple: false } => dialog.pick_file().map(|p| vec![p]),
        FileDialogKind::Open { multiple: true } => dialog.pick_files(),
        FileDialogKind::Save => dialog.save_file().map(|p| vec![p]),
        FileDialogKind::Folder { multiple: false } => dialog.pick_folder().map(|p| vec![p]),
        FileDialogKind::Folder { multiple: true } => dialog.pick_folders(),
    }?;
    Some(paths.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
}

fn build_file_dialog(params: &DialogParams) -> rfd::FileDialog {
    let mut d = rfd::FileDialog::new();
    if let Some(ref title) = params.title {
        d = d.set_title(title);
    }
    if let Some(ref path) = params.default_path {
        let p = std::path::Path::new(path);
        if p.is_dir() {
            d = d.set_directory(p);
        } else {
            if let Some(parent) = p.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                d = d.set_directory(parent);
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                d = d.set_file_name(name);
            }
        }
    }
    for (name, exts) in &params.filters {
        let ext_refs: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
        d = d.add_filter(name, &ext_refs);
    }
    d
}

/// "Name|ext1,ext2;Name2|ext3"
pub fn parse_filters(filters: Option<String>) -> Vec<(String, Vec<String>)> {
    let Some(s) = filters.filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    s.split(';')
        .filter_map(|group| {
            let (name, exts) = group.split_once('|')?;
            let extensions = exts.split(',').map(|e| e.trim().to_string()).collect();
            Some((name.to_string(), extensions))
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn init_com() {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

// ---------------------------------------------------------------------------
// Message box (the values of Photino's ShowMessage)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageButtons {
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    RetryCancel,
    AbortRetryIgnore,
}

impl MessageButtons {
    pub fn from_i32(buttons: i32) -> Self {
        match buttons {
            1 => Self::OkCancel,
            2 => Self::YesNo,
            3 => Self::YesNoCancel,
            4 => Self::RetryCancel,
            5 => Self::AbortRetryIgnore,
            _ => Self::Ok,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageIcon {
    Info,
    Warning,
    Error,
    Question,
}

impl MessageIcon {
    pub fn from_i32(icon: i32) -> Self {
        match icon {
            1 => Self::Warning,
            2 => Self::Error,
            3 => Self::Question,
            _ => Self::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageResult {
    Cancel = -1,
    Ok = 0,
    Yes = 1,
    No = 2,
    Abort = 3,
    Retry = 4,
    Ignore = 5,
}

pub fn message(
    instance: Option<&RustinoWindow>,
    title: String,
    text: String,
    buttons: MessageButtons,
    icon: MessageIcon,
) -> MessageResult {
    let result = match instance {
        Some(inst) if inst.is_running() => inst
            .invoke(move |window, _| show_message(&title, &text, buttons, icon, Some(window)))
            .unwrap_or(MessageResult::Cancel),
        _ => show_message(&title, &text, buttons, icon, None),
    };
    // Closing a dialog with only OK acknowledges it (Windows returns OK, GTK a cancel)
    if buttons == MessageButtons::Ok { MessageResult::Ok } else { result }
}

#[cfg(target_os = "windows")]
fn show_message(
    title: &str,
    text: &str,
    buttons: MessageButtons,
    icon: MessageIcon,
    parent: Option<&Window>,
) -> MessageResult {
    use tao::platform::windows::WindowExtWindows;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::HSTRING;

    // MessageBoxW has every button set of Photino, with localized labels (rfd's custom labels
    // need comctl32 v6, which .NET executables don't load)
    let style = match buttons {
        MessageButtons::Ok => MB_OK,
        MessageButtons::OkCancel => MB_OKCANCEL,
        MessageButtons::YesNo => MB_YESNO,
        MessageButtons::YesNoCancel => MB_YESNOCANCEL,
        MessageButtons::RetryCancel => MB_RETRYCANCEL,
        MessageButtons::AbortRetryIgnore => MB_ABORTRETRYIGNORE,
    } | match icon {
        MessageIcon::Info => MB_ICONINFORMATION,
        MessageIcon::Warning => MB_ICONWARNING,
        MessageIcon::Error => MB_ICONERROR,
        MessageIcon::Question => MB_ICONQUESTION,
    };
    let owner = parent.map(|window| HWND(window.hwnd() as _));
    let result = unsafe { MessageBoxW(owner, &HSTRING::from(text), &HSTRING::from(title), style) };
    match result {
        IDOK => MessageResult::Ok,
        IDYES => MessageResult::Yes,
        IDNO => MessageResult::No,
        IDABORT => MessageResult::Abort,
        IDRETRY => MessageResult::Retry,
        IDIGNORE => MessageResult::Ignore,
        _ => MessageResult::Cancel,
    }
}

#[cfg(target_os = "macos")]
fn show_message(
    title: &str,
    text: &str,
    buttons: MessageButtons,
    icon: MessageIcon,
    parent: Option<&Window>,
) -> MessageResult {
    use rfd::{MessageButtons as Buttons, MessageDialogResult, MessageLevel};

    let rfd_buttons = match buttons {
        MessageButtons::Ok => Buttons::Ok,
        MessageButtons::OkCancel => Buttons::OkCancel,
        MessageButtons::YesNo => Buttons::YesNo,
        MessageButtons::YesNoCancel => Buttons::YesNoCancel,
        MessageButtons::RetryCancel => Buttons::OkCancelCustom("Retry".into(), "Cancel".into()),
        MessageButtons::AbortRetryIgnore => {
            Buttons::YesNoCancelCustom("Abort".into(), "Retry".into(), "Ignore".into())
        }
    };
    // NSAlert has no question style
    let level = match icon {
        MessageIcon::Warning => MessageLevel::Warning,
        MessageIcon::Error => MessageLevel::Error,
        MessageIcon::Info | MessageIcon::Question => MessageLevel::Info,
    };
    let mut dialog = rfd::MessageDialog::new()
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd_buttons)
        .set_level(level);
    if let Some(window) = parent {
        dialog = dialog.set_parent(window);
    }
    match dialog.show() {
        MessageDialogResult::Ok => MessageResult::Ok,
        MessageDialogResult::Yes => MessageResult::Yes,
        MessageDialogResult::No => MessageResult::No,
        MessageDialogResult::Cancel => MessageResult::Cancel,
        MessageDialogResult::Custom(label) => match label.as_str() {
            "Abort" => MessageResult::Abort,
            "Retry" => MessageResult::Retry,
            "Ignore" => MessageResult::Ignore,
            _ => MessageResult::Cancel,
        },
    }
}

#[cfg(target_os = "linux")]
fn show_message(
    title: &str,
    text: &str,
    buttons: MessageButtons,
    icon: MessageIcon,
    parent: Option<&Window>,
) -> MessageResult {
    use tao::platform::unix::WindowExtUnix;
    // rfd's portal backend shows message boxes with zenity, which may not be installed
    match parent {
        Some(window) => linux::message(title, text, buttons, icon, Some(window.gtk_window())),
        // GTK can run on one thread only: without a window, the one that initializes it
        None if gtk::is_initialized_main_thread() || (!gtk::is_initialized() && gtk::init().is_ok()) => {
            linux::message(title, text, buttons, icon, None)
        }
        None => MessageResult::Cancel,
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use gtk::glib;
    use gtk::prelude::*;
    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
        WindowHandle,
    };

    use super::{MessageButtons, MessageIcon, MessageResult};

    /// The parent of a portal dialog. tao opens a new X11 connection for every display handle,
    /// and the portal needs the display only on Wayland.
    pub struct DialogParent {
        window: RawWindowHandle,
        display: Option<RawDisplayHandle>,
    }

    impl DialogParent {
        pub fn of(window: &tao::window::Window) -> Option<Self> {
            let raw = window.window_handle().ok()?.as_raw();
            let display = match raw {
                RawWindowHandle::Wayland(_) => Some(window.display_handle().ok()?.as_raw()),
                _ => None,
            };
            Some(Self { window: raw, display })
        }
    }

    impl HasWindowHandle for DialogParent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Ok(unsafe { WindowHandle::borrow_raw(self.window) })
        }
    }

    impl HasDisplayHandle for DialogParent {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            let display = self.display.ok_or(HandleError::Unavailable)?;
            Ok(unsafe { DisplayHandle::borrow_raw(display) })
        }
    }

    /// Runs `task` on another thread; GTK keeps handling events until it's done.
    pub fn wait_without_blocking_gtk<R: Send + 'static>(task: impl FnOnce() -> R + Send + 'static) -> Option<R> {
        struct QuitOnDrop(glib::MainLoop);
        impl Drop for QuitOnDrop {
            fn drop(&mut self) {
                let main_loop = self.0.clone();
                glib::MainContext::default().invoke(move || main_loop.quit());
            }
        }

        let main_loop = glib::MainLoop::new(None, false);
        let (tx, rx) = std::sync::mpsc::channel();
        let quit = QuitOnDrop(main_loop.clone());
        std::thread::spawn(move || {
            // Also when the task panics
            let _quit = quit;
            let _ = tx.send(task());
        });
        main_loop.run();
        rx.recv().ok()
    }

    pub fn message(
        title: &str,
        text: &str,
        buttons: MessageButtons,
        icon: MessageIcon,
        parent: Option<&gtk::ApplicationWindow>,
    ) -> MessageResult {
        let message_type = match icon {
            MessageIcon::Info => gtk::MessageType::Info,
            MessageIcon::Warning => gtk::MessageType::Warning,
            MessageIcon::Error => gtk::MessageType::Error,
            MessageIcon::Question => gtk::MessageType::Question,
        };
        // GNOME shows no title bar on message dialogs: the title is the primary text
        let dialog = gtk::MessageDialog::new(
            parent,
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            message_type,
            gtk::ButtonsType::None,
            title,
        );
        dialog.set_title(title);
        if !text.is_empty() {
            dialog.set_secondary_text(Some(text));
        }
        let responses = responses(buttons);
        for (label, result) in &responses {
            // GTK's own translations of its button labels
            let label = glib::dgettext(Some("gtk30"), label);
            dialog.add_button(&label, response_type(*result));
        }
        if let Some((_, first)) = responses.first() {
            dialog.set_default_response(response_type(*first));
        }
        let response = dialog.run();
        dialog.close();
        responses
            .iter()
            .map(|(_, result)| *result)
            .find(|result| response_type(*result) == response)
            .unwrap_or(MessageResult::Cancel)
    }

    fn responses(buttons: MessageButtons) -> Vec<(&'static str, MessageResult)> {
        use MessageResult::*;
        match buttons {
            MessageButtons::Ok => vec![("_OK", Ok)],
            MessageButtons::OkCancel => vec![("_OK", Ok), ("_Cancel", Cancel)],
            MessageButtons::YesNo => vec![("_Yes", Yes), ("_No", No)],
            MessageButtons::YesNoCancel => vec![("_Yes", Yes), ("_No", No), ("_Cancel", Cancel)],
            MessageButtons::RetryCancel => vec![("_Retry", Retry), ("_Cancel", Cancel)],
            MessageButtons::AbortRetryIgnore => vec![("_Abort", Abort), ("_Retry", Retry), ("_Ignore", Ignore)],
        }
    }

    fn response_type(result: MessageResult) -> gtk::ResponseType {
        match result {
            MessageResult::Ok => gtk::ResponseType::Ok,
            MessageResult::Cancel => gtk::ResponseType::Cancel,
            MessageResult::Yes => gtk::ResponseType::Yes,
            MessageResult::No => gtk::ResponseType::No,
            other => gtk::ResponseType::Other(other as u16),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_parse() {
        let filters = parse_filters(Some("Images|png, jpg;All|*".into()));
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0], ("Images".to_string(), vec!["png".to_string(), "jpg".to_string()]));
        assert_eq!(filters[1].1, vec!["*".to_string()]);
        assert!(parse_filters(None).is_empty());
        assert!(parse_filters(Some(String::new())).is_empty());
        assert!(parse_filters(Some("no separator".into())).is_empty());
    }

    #[test]
    fn message_values_match_the_host_enums() {
        assert_eq!(MessageButtons::from_i32(4), MessageButtons::RetryCancel);
        assert_eq!(MessageButtons::from_i32(5), MessageButtons::AbortRetryIgnore);
        assert_eq!(MessageButtons::from_i32(99), MessageButtons::Ok);
        assert_eq!(MessageIcon::from_i32(3), MessageIcon::Question);
        assert_eq!(MessageIcon::from_i32(-1), MessageIcon::Info);
        assert_eq!(MessageResult::Cancel as i32, -1);
        assert_eq!(MessageResult::Ignore as i32, 5);
    }
}
