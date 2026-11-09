//! The commands of a running window, on its event loop thread.

use std::ffi::CString;
use std::sync::atomic::Ordering;

use tao::dpi::{PhysicalPosition, PhysicalSize};

use crate::badge::set_badge_count;
use crate::callbacks::RustinoCallbacks;
use crate::commands::RustinoCommand;
use crate::config::WindowConfig;
use crate::icon;
use crate::menu::{self, about_metadata, remove_menu_from_window, set_menu_bar, show_context_menu};
use crate::state::SharedState;
use crate::tray::Tray;

use super::runtime::center_window;

/// The host can't get the result of a queued command: tell its logger when one fails.
fn report_failure(config: &WindowConfig, operation: &str, result: wry::Result<()>) {
    if let Err(e) = result {
        config.logger().error(&format!("[rustino] {operation} failed: {e}"));
    }
}

pub(super) fn dispatch_command(
    cmd: RustinoCommand,
    window: &tao::window::Window,
    webview: &wry::WebView,
    state: &SharedState,
    callbacks: RustinoCallbacks,
    menu_items: &mut menu::MenuItems,
    current_menu: &mut Option<muda::Menu>,
    config: &mut WindowConfig,
    tray: &mut Option<Tray>,
) -> bool {
    match cmd {
        RustinoCommand::SetTitle(title) => window.set_title(&title),
        RustinoCommand::SetSize(w, h) => {
            window.set_inner_size(PhysicalSize::new(w, h));
        }
        RustinoCommand::SetMinimized(v) => {
            window.set_minimized(v);
            state.is_minimized.store(v, Ordering::Release);
        }
        RustinoCommand::SetMaximized(v) => {
            window.set_maximized(v);
            state.is_maximized.store(v, Ordering::Release);
        }
        RustinoCommand::SetFullscreen(v) => {
            if v {
                window.set_fullscreen(Some(tao::window::Fullscreen::Borderless(None)));
            } else {
                window.set_fullscreen(None);
            }
            state.is_fullscreen.store(v, Ordering::Release);
        }
        RustinoCommand::SetVisible(v) => {
            window.set_visible(v);
            state.is_visible.store(v, Ordering::Release);
        }
        RustinoCommand::SetFocus => window.set_focus(),
        RustinoCommand::SetDecorations(v) => window.set_decorations(v),
        RustinoCommand::SetPosition(x, y) => {
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
        RustinoCommand::Center => center_window(window),
        RustinoCommand::SetMinSize(size) => {
            window.set_min_inner_size(size.map(|(w, h)| PhysicalSize::new(w, h)));
        }
        RustinoCommand::SetMaxSize(size) => {
            window.set_max_inner_size(size.map(|(w, h)| PhysicalSize::new(w, h)));
        }
        RustinoCommand::SetResizable(v) => window.set_resizable(v),
        RustinoCommand::SetTopmost(v) => window.set_always_on_top(v),
        RustinoCommand::SetIconFile(path) => {
            if let Some(ico) = icon::load_icon(&path) {
                window.set_window_icon(Some(ico));
            }
            config.icon_file = Some(path);
        }
        RustinoCommand::SetAbout(field, value) => config.set_about(field, value),
        RustinoCommand::EvaluateScript(js) => {
            report_failure(config, "ExecuteScript", webview.evaluate_script(&js));
        }
        RustinoCommand::SendWebMessage(msg) => {
            let data = crate::util::js_string_literal(&msg);
            let js = format!("window.dispatchEvent(new MessageEvent('message',{{data:{data}}}));");
            report_failure(config, "SendWebMessage", webview.evaluate_script(&js));
        }
        RustinoCommand::LoadUrl(url) => {
            report_failure(config, "Load", webview.load_url(&url));
            config.start_html = None;
        }
        RustinoCommand::LoadHtml(html) => {
            report_failure(config, "Load", webview.load_html(&html));
            config.start_html = Some(html);
        }
        RustinoCommand::Reload => {
            // A page loaded from a string has no URL: reloading it would show about:blank
            let result = match &config.start_html {
                Some(html) if webview.url().is_ok_and(|url| url == "about:blank") => webview.load_html(html),
                _ => webview.reload(),
            };
            report_failure(config, "Reload", result);
        }
        RustinoCommand::SetZoom(factor) => {
            report_failure(config, "SetZoom", webview.zoom(factor));
        }
        RustinoCommand::SetBackgroundColor(r, g, b, a) => {
            report_failure(config, "SetBackgroundColor", webview.set_background_color((r, g, b, a)));
        }
        RustinoCommand::Window(_) => {} // run by window_ext
        RustinoCommand::Invoke(task) => task.run(window, webview),
        RustinoCommand::SetBadgeCount { count, bg_r, bg_g, bg_b, fg_r, fg_g, fg_b } => {
            set_badge_count(window, count, [bg_r, bg_g, bg_b], [fg_r, fg_g, fg_b]);
        }
        RustinoCommand::SetMenu(json) => {
            let about = about_metadata(config);
            if let Some(built) = menu::build_menu(&json, &about) {
                set_menu_bar(built, window, config, &about, current_menu, menu_items);
            }
        }
        RustinoCommand::RemoveMenu => {
            if let Some(old) = current_menu.take() {
                remove_menu_from_window(&old, window);
            }
            menu_items.replace(menu::MenuOwner::MenuBar, Vec::new());
        }
        RustinoCommand::ShowContextMenu(json, pos) => {
            if let Some(built) = menu::build_menu(&json, &about_metadata(config)) {
                menu_items.replace(menu::MenuOwner::ContextMenu, built.items);
                show_context_menu(&built.menu, window, pos);
            }
        }
        RustinoCommand::UpdateMenuItem(id, update) => menu_items.update(&id, &update),
        RustinoCommand::SetTrayIcon(params) => {
            *tray = None;
            menu_items.replace(menu::MenuOwner::Tray, Vec::new());
            *tray = Tray::build(&params, window, config, menu_items);
        }
        RustinoCommand::SetTrayTitle(title) => {
            if let Some(tray) = tray {
                // tray-icon ignores `None` on macOS: an empty title removes it everywhere
                tray.icon.set_title(Some(title.unwrap_or_default()));
            }
        }
        RustinoCommand::RemoveTrayIcon => {
            *tray = None;
            menu_items.replace(menu::MenuOwner::Tray, Vec::new());
        }
        // Menu and tray events reach every window: each handles only its own items and tray icon
        RustinoCommand::MenuEventFired(menu_id) => {
            if let Some((id, checked)) = menu_items.clicked(&menu_id)
                && let Some(cb) = callbacks.on_menu_item_clicked
                && let Ok(cstr) = CString::new(id)
            {
                unsafe { cb(callbacks.context, cstr.as_ptr(), checked.map_or(-1, i32::from)) };
            }
        }
        RustinoCommand::TrayIconButton {
            id,
            button,
            pressed,
            x,
            y,
        } => {
            // One click event per click: on press when the click opens the tray menu (on macOS
            // the menu takes the release), otherwise on release
            if let Some(tray) = tray.as_ref()
                && *tray.icon.id() == id
                && pressed == tray.opens_menu(button)
                && let Some(cb) = callbacks.on_tray_icon_clicked
            {
                let button = match button {
                    tray_icon::MouseButton::Left => 0,
                    tray_icon::MouseButton::Right => 1,
                    tray_icon::MouseButton::Middle => 2,
                };
                unsafe { cb(callbacks.context, button, x, y) };
            }
        }
        // Handled by WindowRuntime::handle
        RustinoCommand::GlobalShortcutPressed(_) => {}
        RustinoCommand::Close => return true,
    }
    false
}
