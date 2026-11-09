//! Rustino's native library: windows with a webview (tao + wry), behind a C API for .NET.
//!
//! Each feature keeps its C API (the `#[no_mangle] extern "C"` functions, named `rustino_*`)
//! next to its implementation: at the top of its file, or in `exports.rs` when the feature is a
//! directory. Rustino.NET declares the same functions in `src/Rustino.NET/Interop`, and the
//! interop tests of `tests/Rustino.NET.Tests` compare the two sides.
//!
//! - `window`: the window, the event loop of the app and their C API
//! - `window_ext`, `webview_ext`, `menu`, `tray`, `dialogs`, `badge`, `monitors`, ...: what a
//!   window can do
//! - `autostart`, `safe_storage`, `notifications`, `splash`, `external`: no window needed
//! - `util`: strings across the C boundary

#[cfg(test)]
mod abi_layout;
mod activate;
mod autostart;
mod badge;
mod callbacks;
mod commands;
mod config;
mod custom_scheme;
mod deep_links;
mod dialogs;
mod external;
mod global_shortcuts;
mod icon;
mod invoke;
mod menu;
mod monitors;
mod notifications;
mod parent;
mod safe_storage;
mod splash;
mod state;
mod tray;
mod util;
mod webview_ext;
mod window;
mod window_ext;
