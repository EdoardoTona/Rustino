use std::sync::mpsc;

use crate::config::AboutField;
use crate::menu::MenuItemUpdate;

#[derive(Debug)]
pub enum RustinoCommand {
    SetTitle(String),
    SetSize(u32, u32),
    SetMinimized(bool),
    SetMaximized(bool),
    SetFullscreen(bool),
    SetVisible(bool),
    SetFocus,
    SetDecorations(bool),
    SetPosition(i32, i32),
    Center,
    SetMinSize(Option<(u32, u32)>),
    SetMaxSize(Option<(u32, u32)>),
    SetResizable(bool),
    SetTopmost(bool),
    SetIconFile(String),
    SetAbout(AboutField, Option<String>),

    EvaluateScript(String),
    SendWebMessage(String),
    LoadUrl(String),
    LoadHtml(String),
    SetZoom(f64),
    Reload,
    SetBackgroundColor(u8, u8, u8, u8),
    /// Run by window_ext before dispatch_command
    Window(crate::window_ext::WindowCommand),
    /// Code that needs the event loop thread (see invoke)
    Invoke(crate::invoke::Task),

    SetMenu(String),
    RemoveMenu,
    ShowContextMenu(String, Option<(f64, f64)>),
    UpdateMenuItem(String, MenuItemUpdate),
    SetTrayIcon(TrayParams),
    SetTrayTitle(Option<String>),
    RemoveTrayIcon,

    SetBadgeCount {
        count: Option<u32>,
        bg_r: u8,
        bg_g: u8,
        bg_b: u8,
        fg_r: u8,
        fg_g: u8,
        fg_b: u8,
    },

    GetMonitors(mpsc::Sender<String>),
    GetCurrentMonitor(mpsc::Sender<String>),

    MenuEventFired(muda::MenuId),
    TrayIconButton {
        id: tray_icon::TrayIconId,
        button: tray_icon::MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    },

    Close,
}

#[derive(Debug)]
pub struct TrayParams {
    pub icon_path: String,
    pub tooltip: Option<String>,
    pub menu_json: Option<String>,
    pub title: Option<String>,
    pub icon_is_template: bool,
    pub menu_on_left_click: bool,
}
