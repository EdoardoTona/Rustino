use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    AboutMetadata, CheckMenuItem, IconMenuItem, IsMenuItem, Menu, MenuId, MenuItem, MenuItemKind,
    PredefinedMenuItem, Submenu,
};
use serde::Deserialize;

use crate::icon;

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum MenuItemDef {
    #[serde(rename = "normal")]
    Normal {
        id: String,
        label: String,
        accelerator: Option<String>,
        enabled: Option<bool>,
        icon: Option<String>,
    },
    #[serde(rename = "check")]
    Check {
        id: String,
        label: String,
        accelerator: Option<String>,
        checked: Option<bool>,
        enabled: Option<bool>,
    },
    #[serde(rename = "separator")]
    Separator,
    #[serde(rename = "predefined")]
    Predefined { item: String, label: Option<String> },
    #[serde(rename = "submenu")]
    Submenu {
        label: String,
        enabled: Option<bool>,
        role: Option<SubmenuRole>,
        items: Vec<MenuItemDef>,
    },
    #[serde(rename = "app_menu")]
    AppMenu { items: Vec<MenuItemDef> },
}

/// Submenus that macOS fills itself: the open windows in the Window menu, a search field in
/// the Help menu. Plain submenus on Windows and Linux.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SubmenuRole {
    Window,
    Help,
}

pub struct BuiltMenu {
    pub menu: Menu,
    /// macOS application menu from AddAppMenu (always `None` on other platforms)
    pub app_menu: Option<Submenu>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub window_menu: Option<Submenu>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub help_menu: Option<Submenu>,
    /// Items with a host id, for click events and runtime changes
    pub items: Vec<(String, MenuItemKind)>,
    /// Ctrl+C/X/V/A/Z/Y of custom items, as bits of `EDIT_SHORTCUT_KEYS` (used on Windows)
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub custom_edit_shortcuts: u8,
}

pub fn build_menu(json: &str, about: &AboutMetadata) -> Option<BuiltMenu> {
    let defs: Vec<MenuItemDef> = serde_json::from_str(json).ok()?;
    let mut built = BuiltMenu {
        menu: Menu::new(),
        app_menu: None,
        window_menu: None,
        help_menu: None,
        items: Vec::new(),
        custom_edit_shortcuts: custom_edit_shortcuts(&defs),
    };
    built.app_menu = defs
        .iter()
        .find_map(|def| match def {
            MenuItemDef::AppMenu { items } if cfg!(target_os = "macos") => Some(items),
            _ => None,
        })
        .map(|items| {
            let submenu = Submenu::new("App", true);
            append_items(&|item| submenu.append(item), items, &mut built, about);
            submenu
        });
    let menu = built.menu.clone();
    append_items(&|item| menu.append(item), &defs, &mut built, about);
    Some(built)
}

/// Builds top-down: on Windows muda registers an item's accelerator only if its
/// submenu is already attached to the menu.
fn append_items(
    append: &dyn Fn(&dyn IsMenuItem) -> muda::Result<()>,
    defs: &[MenuItemDef],
    built: &mut BuiltMenu,
    about: &AboutMetadata,
) {
    for def in defs {
        if let MenuItemDef::Submenu {
            label,
            enabled,
            role,
            items,
        } = def
        {
            let submenu = Submenu::new(label, enabled.unwrap_or(true));
            let _ = append(&submenu);
            append_items(&|item| submenu.append(item), items, built, about);
            match role {
                Some(SubmenuRole::Window) => built.window_menu = Some(submenu),
                Some(SubmenuRole::Help) => built.help_menu = Some(submenu),
                None => {}
            }
        } else if let Some(item) = build_item(def, &mut built.items, about) {
            let _ = append(item.as_ref());
        }
    }
}

fn build_item(
    def: &MenuItemDef,
    items: &mut Vec<(String, MenuItemKind)>,
    about: &AboutMetadata,
) -> Option<Box<dyn IsMenuItem>> {
    let parse_accelerator = |accelerator: &Option<String>| {
        accelerator
            .as_deref()
            .and_then(|a| a.parse::<Accelerator>().ok())
    };
    match def {
        MenuItemDef::Normal {
            id,
            label,
            accelerator,
            enabled,
            icon: icon_path,
        } => {
            let (accel, enabled) = (parse_accelerator(accelerator), enabled.unwrap_or(true));
            // An unreadable icon leaves a plain item
            let item: Box<dyn IsMenuItem> = match icon_path.as_deref().and_then(icon::load_menu_icon) {
                Some(icon) => Box::new(IconMenuItem::with_id(
                    unique_id(),
                    label,
                    enabled,
                    Some(icon),
                    accel,
                )),
                None => Box::new(MenuItem::with_id(unique_id(), label, enabled, accel)),
            };
            items.push((id.clone(), item.kind()));
            Some(item)
        }
        MenuItemDef::Check {
            id,
            label,
            accelerator,
            checked,
            enabled,
        } => {
            let item = CheckMenuItem::with_id(
                unique_id(),
                label,
                enabled.unwrap_or(true),
                checked.unwrap_or(false),
                parse_accelerator(accelerator),
            );
            items.push((id.clone(), item.kind()));
            Some(Box::new(item))
        }
        MenuItemDef::Separator => Some(Box::new(PredefinedMenuItem::separator())),
        // Native OS actions (e.g. Copy/Paste reach the focused webview); they fire no menu events
        MenuItemDef::Predefined { item, label } => {
            let label = label.as_deref();
            let item = match item.as_str() {
                "about" => PredefinedMenuItem::about(label, Some(about.clone())),
                "undo" => PredefinedMenuItem::undo(label),
                "redo" => PredefinedMenuItem::redo(label),
                "cut" => PredefinedMenuItem::cut(label),
                "copy" => PredefinedMenuItem::copy(label),
                "paste" => PredefinedMenuItem::paste(label),
                "select_all" => PredefinedMenuItem::select_all(label),
                "minimize" => PredefinedMenuItem::minimize(label),
                "maximize" => PredefinedMenuItem::maximize(label),
                "fullscreen" => PredefinedMenuItem::fullscreen(label),
                "hide" => PredefinedMenuItem::hide(label),
                "hide_others" => PredefinedMenuItem::hide_others(label),
                "show_all" => PredefinedMenuItem::show_all(label),
                "close_window" => PredefinedMenuItem::close_window(label),
                "quit" => PredefinedMenuItem::quit(label),
                "services" => PredefinedMenuItem::services(label),
                "bring_all_to_front" => PredefinedMenuItem::bring_all_to_front(label),
                _ => return None,
            };
            Some(Box::new(item))
        }
        // Built by append_items / build_menu
        MenuItemDef::Submenu { .. } | MenuItemDef::AppMenu { .. } => None,
    }
}

/// Keys of Ctrl+key edit shortcuts: those of the predefined Copy, Cut, Paste, Select All, Undo
/// and Redo items on Windows, which re-send their shortcut to the focused control.
pub const EDIT_SHORTCUT_KEYS: [u8; 6] = *b"CXVAZY";

/// The bit of an `EDIT_SHORTCUT_KEYS` key (a virtual key code on Windows), 0 for other keys.
pub fn edit_shortcut_bit(key: u8) -> u8 {
    EDIT_SHORTCUT_KEYS
        .iter()
        .position(|&k| k == key)
        .map_or(0, |i| 1 << i)
}

/// The edit shortcuts of the custom items, without those of the menu's predefined items: a
/// predefined item re-sends its shortcut, which must reach the focused control.
fn custom_edit_shortcuts(defs: &[MenuItemDef]) -> u8 {
    fn collect(defs: &[MenuItemDef], custom: &mut u8, predefined: &mut u8) {
        for def in defs {
            match def {
                MenuItemDef::Normal { accelerator, .. } | MenuItemDef::Check { accelerator, .. } => {
                    let accelerator = accelerator.as_deref().and_then(|a| a.parse::<Accelerator>().ok());
                    if let Some(accelerator) = accelerator
                        && accelerator.modifiers() == Modifiers::CONTROL
                    {
                        let key = match accelerator.key() {
                            Code::KeyC => b'C',
                            Code::KeyX => b'X',
                            Code::KeyV => b'V',
                            Code::KeyA => b'A',
                            Code::KeyZ => b'Z',
                            Code::KeyY => b'Y',
                            _ => continue,
                        };
                        *custom |= edit_shortcut_bit(key);
                    }
                }
                MenuItemDef::Predefined { item, .. } => {
                    let key = match item.as_str() {
                        "copy" => b'C',
                        "cut" => b'X',
                        "paste" => b'V',
                        "select_all" => b'A',
                        "undo" => b'Z',
                        "redo" => b'Y',
                        _ => continue,
                    };
                    *predefined |= edit_shortcut_bit(key);
                }
                MenuItemDef::Submenu { items, .. } => collect(items, custom, predefined),
                // The application menu exists only on macOS
                MenuItemDef::AppMenu { .. } | MenuItemDef::Separator => {}
            }
        }
    }
    let (mut custom, mut predefined) = (0, 0);
    collect(defs, &mut custom, &mut predefined);
    custom & !predefined
}

/// muda id of a host item: unique, because the same host id can be used in several menus
/// (e.g. the menu bar and the tray menu). Prefixed so it can't clash with muda's own ids.
fn unique_id() -> MenuId {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    MenuId::new(format!("rustino-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// The menu an item belongs to: setting a menu replaces the items of the previous one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuOwner {
    MenuBar,
    Tray,
    ContextMenu,
}

#[derive(Debug)]
pub enum MenuItemUpdate {
    Enabled(bool),
    Checked(bool),
    Text(String),
}

/// The host items of a window's menus, by muda id. Lives on the event loop thread.
#[derive(Default)]
pub struct MenuItems {
    items: HashMap<MenuId, (MenuOwner, String, MenuItemKind)>,
}

impl MenuItems {
    pub fn replace(&mut self, owner: MenuOwner, items: Vec<(String, MenuItemKind)>) {
        self.items.retain(|_, (item_owner, ..)| *item_owner != owner);
        self.items.extend(
            items
                .into_iter()
                .map(|(id, item)| (item.id().clone(), (owner, id, item))),
        );
    }

    /// Host id of a clicked item and, for check items, the state muda just toggled, which is
    /// also applied to the items with the same host id. `None` for items of other windows.
    pub fn clicked(&self, menu_id: &MenuId) -> Option<(String, Option<bool>)> {
        let (_, id, item) = self.items.get(menu_id)?;
        let checked = item.as_check_menuitem().map(CheckMenuItem::is_checked);
        if let Some(checked) = checked {
            self.update(id, &MenuItemUpdate::Checked(checked));
        }
        Some((id.clone(), checked))
    }

    /// Applies a change to every item with this host id in the menu bar and in the tray menu.
    /// Context menus are built from their definition at every `ShowContextMenu`.
    pub fn update(&self, id: &str, update: &MenuItemUpdate) {
        for (owner, item_id, item) in self.items.values() {
            if *owner != MenuOwner::ContextMenu && item_id == id {
                apply_update(item, update);
            }
        }
    }
}

fn apply_update(item: &MenuItemKind, update: &MenuItemUpdate) {
    match (item, update) {
        (MenuItemKind::MenuItem(i), MenuItemUpdate::Enabled(enabled)) => i.set_enabled(*enabled),
        (MenuItemKind::Icon(i), MenuItemUpdate::Enabled(enabled)) => i.set_enabled(*enabled),
        (MenuItemKind::Check(i), MenuItemUpdate::Enabled(enabled)) => i.set_enabled(*enabled),
        (MenuItemKind::Check(i), MenuItemUpdate::Checked(checked)) => i.set_checked(*checked),
        (MenuItemKind::MenuItem(i), MenuItemUpdate::Text(text)) => i.set_text(text),
        (MenuItemKind::Icon(i), MenuItemUpdate::Text(text)) => i.set_text(text),
        (MenuItemKind::Check(i), MenuItemUpdate::Text(text)) => i.set_text(text),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{custom_edit_shortcuts, edit_shortcut_bit, MenuItemDef, SubmenuRole};

    fn predefined(json: &str) -> (String, Option<String>) {
        match serde_json::from_str(json).unwrap() {
            MenuItemDef::Predefined { item, label } => (item, label),
            _ => panic!("not a predefined item"),
        }
    }

    #[test]
    fn parses_predefined_item() {
        let copy = predefined(r#"{"type":"predefined","item":"copy","label":"Copy"}"#);
        assert_eq!(copy, ("copy".into(), Some("Copy".into())));
        let quit = predefined(r#"{"type":"predefined","item":"quit"}"#);
        assert_eq!(quit, ("quit".into(), None));
    }

    #[test]
    fn parses_check_item_accelerator() {
        let json = r#"{"type":"check","id":"wrap","label":"Wrap","accelerator":"CmdOrCtrl+W"}"#;
        match serde_json::from_str(json).unwrap() {
            MenuItemDef::Check {
                accelerator,
                checked,
                ..
            } => {
                assert_eq!(accelerator.as_deref(), Some("CmdOrCtrl+W"));
                assert_eq!(checked, None);
            }
            _ => panic!("not a check item"),
        }
    }

    #[test]
    fn parses_submenu_role() {
        let role = |json: &str| match serde_json::from_str(json).unwrap() {
            MenuItemDef::Submenu { role, .. } => role,
            _ => panic!("not a submenu"),
        };
        let window = r#"{"type":"submenu","label":"Window","role":"window","items":[]}"#;
        assert_eq!(role(window), Some(SubmenuRole::Window));
        let help = r#"{"type":"submenu","label":"Help","role":"help","items":[]}"#;
        assert_eq!(role(help), Some(SubmenuRole::Help));
        assert_eq!(role(r#"{"type":"submenu","label":"File","items":[]}"#), None);
    }

    #[test]
    fn custom_edit_shortcuts_skip_predefined_ones() {
        let shortcuts = |json: &str| custom_edit_shortcuts(&serde_json::from_str::<Vec<MenuItemDef>>(json).unwrap());
        let (c, y, z) = (edit_shortcut_bit(b'C'), edit_shortcut_bit(b'Y'), edit_shortcut_bit(b'Z'));
        // Custom Ctrl+Y and Ctrl+C in a submenu; Ctrl+Shift+Z and Alt+V are not edit shortcuts
        let custom = r#"[{"type":"submenu","label":"Edit","items":[
            {"type":"normal","id":"redo","label":"Redo","accelerator":"Ctrl+Y"},
            {"type":"check","id":"copy","label":"Copy","accelerator":"Ctrl+C"},
            {"type":"normal","id":"z","label":"Z","accelerator":"Ctrl+Shift+Z"},
            {"type":"normal","id":"v","label":"V","accelerator":"Alt+V"}]}]"#;
        assert_eq!(shortcuts(custom), y | c);
        // A predefined item with the same shortcut keeps it for the focused control
        let conflict = r#"[{"type":"submenu","label":"Edit","items":[
            {"type":"predefined","item":"undo"},
            {"type":"normal","id":"z","label":"Z","accelerator":"Ctrl+Z"},
            {"type":"normal","id":"y","label":"Y","accelerator":"Ctrl+Y"}]}]"#;
        assert_eq!(shortcuts(conflict), y);
        assert_eq!(z & shortcuts(conflict), 0);
        assert_eq!(edit_shortcut_bit(b'B'), 0);
    }
}
