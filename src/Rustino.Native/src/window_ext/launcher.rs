//! Badge and progress on the dock icon through the Unity LauncherEntry D-Bus signals, which
//! Ubuntu Dock, Dash to Dock and KDE Plasma follow. tao sends them through libunity only while
//! Unity itself runs.

use std::collections::HashMap;
use std::sync::Mutex;

use gtk::gio;
use gtk::glib::{ToVariant, Variant};
use tao::window::ProgressState;

/// The dock entry is the app's, shared by its windows
static DESKTOP_FILE_NAME: Mutex<Option<String>> = Mutex::new(None);

pub fn set_desktop_file_name(name: Option<String>) {
    if let Ok(mut current) = DESKTOP_FILE_NAME.lock() {
        *current = name;
    }
}

pub fn set_count(count: Option<u32>) {
    let count = count.unwrap_or(0);
    update(&[
        ("count", i64::from(count).to_variant()),
        ("count-visible", (count > 0).to_variant()),
    ]);
}

pub fn set_progress(state: ProgressState, progress: Option<u64>) {
    let visible = !matches!(state, ProgressState::None);
    let mut properties = vec![("progress-visible", visible.to_variant())];
    if let Some(progress) = progress {
        properties.push(("progress", (progress.min(100) as f64 / 100.0).to_variant()));
    }
    update(&properties);
}

fn update(properties: &[(&str, Variant)]) {
    let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return;
    };
    let app_uri = app_uri();
    let properties: HashMap<String, Variant> =
        properties.iter().map(|(key, value)| (key.to_string(), value.clone())).collect();
    let path = format!("/com/canonical/unity/launcherentry/{}", path_id(&app_uri));
    let _ = connection.emit_signal(
        None,
        &path,
        "com.canonical.Unity.LauncherEntry",
        "Update",
        Some(&(app_uri, properties).to_variant()),
    );
}

/// `application://<desktop file>`, by default `<executable name>.desktop`
pub(super) fn app_uri() -> String {
    let name = DESKTOP_FILE_NAME.lock().ok().and_then(|name| name.clone()).unwrap_or_else(|| {
        let exe = std::env::current_exe().ok();
        let stem = exe.as_deref().and_then(|p| p.file_stem()).and_then(|s| s.to_str());
        format!("{}.desktop", stem.unwrap_or("rustino"))
    });
    format!("application://{name}")
}

/// Any object path works: the docks read the app from the signal
pub(super) fn path_id(app_uri: &str) -> u32 {
    app_uri.bytes().fold(5381u32, |hash, b| hash.wrapping_mul(33).wrapping_add(u32::from(b)))
}
