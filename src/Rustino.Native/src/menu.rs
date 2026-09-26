use std::collections::HashMap;

use muda::{
    accelerator::Accelerator, AboutMetadata, CheckMenuItem, IsMenuItem, Menu, MenuId, MenuItem,
    PredefinedMenuItem, Submenu,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum MenuItemDef {
    #[serde(rename = "normal")]
    Normal {
        id: String,
        label: String,
        accelerator: Option<String>,
        enabled: Option<bool>,
    },
    #[serde(rename = "check")]
    Check {
        id: String,
        label: String,
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
        items: Vec<MenuItemDef>,
    },
    #[serde(rename = "app_menu")]
    AppMenu { items: Vec<MenuItemDef> },
}

pub struct BuiltMenu {
    pub menu: Menu,
    /// macOS application menu from AddAppMenu (always `None` on other platforms)
    pub app_menu: Option<Submenu>,
    pub id_map: HashMap<MenuId, String>,
}

pub fn build_menu(json: &str, about: &AboutMetadata) -> Option<BuiltMenu> {
    let defs: Vec<MenuItemDef> = serde_json::from_str(json).ok()?;
    let menu = Menu::new();
    let mut id_map = HashMap::new();
    let app_menu = defs
        .iter()
        .find_map(|def| match def {
            MenuItemDef::AppMenu { items } if cfg!(target_os = "macos") => Some(items),
            _ => None,
        })
        .map(|items| {
            let submenu = Submenu::new("App", true);
            append_items(&|item| submenu.append(item), items, &mut id_map, about);
            submenu
        });
    append_items(&|item| menu.append(item), &defs, &mut id_map, about);
    Some(BuiltMenu {
        menu,
        app_menu,
        id_map,
    })
}

/// Builds top-down: on Windows muda registers an item's accelerator only if its
/// submenu is already attached to the menu.
fn append_items(
    append: &dyn Fn(&dyn IsMenuItem) -> muda::Result<()>,
    defs: &[MenuItemDef],
    id_map: &mut HashMap<MenuId, String>,
    about: &AboutMetadata,
) {
    for def in defs {
        if let MenuItemDef::Submenu {
            label,
            enabled,
            items,
        } = def
        {
            let submenu = Submenu::new(label, enabled.unwrap_or(true));
            let _ = append(&submenu);
            append_items(&|item| submenu.append(item), items, id_map, about);
        } else if let Some(item) = build_item(def, id_map, about) {
            let _ = append(item.as_ref());
        }
    }
}

fn build_item(
    def: &MenuItemDef,
    id_map: &mut HashMap<MenuId, String>,
    about: &AboutMetadata,
) -> Option<Box<dyn IsMenuItem>> {
    match def {
        MenuItemDef::Normal {
            id,
            label,
            accelerator,
            enabled,
        } => {
            let accel = accelerator
                .as_deref()
                .and_then(|a| a.parse::<Accelerator>().ok());
            let item = MenuItem::with_id(MenuId::new(id), label, enabled.unwrap_or(true), accel);
            id_map.insert(item.id().clone(), id.clone());
            Some(Box::new(item))
        }
        MenuItemDef::Check {
            id,
            label,
            checked,
            enabled,
        } => {
            let item = CheckMenuItem::with_id(
                MenuId::new(id),
                label,
                enabled.unwrap_or(true),
                checked.unwrap_or(false),
                None::<Accelerator>,
            );
            id_map.insert(item.id().clone(), id.clone());
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

#[cfg(test)]
mod tests {
    use super::MenuItemDef;

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
}
