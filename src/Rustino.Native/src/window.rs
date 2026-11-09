//! The window as the host holds it: its configuration until it runs, then the commands for its
//! event loop.
//!
//! - `exports`: the C API of the window
//! - `event_loop`: the event loop of the app, which the windows share
//! - `runtime`: a window and its webview in the event loop
//! - `dispatch`: the commands of a running window

mod dispatch;
mod event_loop;
mod exports;
mod runtime;

use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::callbacks::RustinoCallbacks;
use crate::commands::RustinoCommand;
use crate::config::WindowConfig;
use crate::monitors::update_monitor_cache;
use crate::state::SharedState;

pub(crate) use event_loop::send_to_event_loops;
pub use event_loop::{CommandProxy, Wake};
pub(crate) use exports::configure;

pub struct RustinoWindow {
    setup: Mutex<Setup>,
    pub state: Arc<SharedState>,
    /// The host's handle and, while `run` runs, the event loop: the last one to let go frees the
    /// instance, so that the host can destroy it from one of its handlers
    holders: AtomicUsize,
}

/// What the exports change, behind one lock: `run` takes the configuration and installs the
/// event loop atomically, so that a setter either changes the configuration or sends a command.
pub struct Setup {
    pub config: WindowConfig,
    /// Taken by `run`: handlers set afterwards are ignored
    pub callbacks: RustinoCallbacks,
    pub ext: crate::window_ext::WindowExt,
    pub webview_ext: crate::webview_ext::WebViewExt,
    phase: Phase,
    /// Commands sent before the window exists, which its event loop gets first
    pending: Vec<RustinoCommand>,
}

enum Phase {
    /// The configuration can change
    Created,
    /// `run` builds the window and the webview
    Starting,
    Running(CommandProxy),
    Exited,
}

/// What `run` takes from the setup.
pub struct Started {
    config: WindowConfig,
    callbacks: RustinoCallbacks,
    ext: crate::window_ext::WindowExt,
    webview_ext: crate::webview_ext::WebViewExt,
}

impl RustinoWindow {
    pub fn new(config: WindowConfig) -> Self {
        let state = Arc::new(SharedState::new(config.width, config.height));
        Self {
            setup: Mutex::new(Setup {
                config,
                callbacks: RustinoCallbacks::default(),
                ext: Default::default(),
                webview_ext: Default::default(),
                phase: Phase::Created,
                pending: Vec::new(),
            }),
            state,
            holders: AtomicUsize::new(1),
        }
    }

    /// The host destroys the instance: a running window closes.
    pub fn close_for_destroy(&self) {
        let mut setup = self.setup();
        match &setup.phase {
            Phase::Starting => setup.pending.push(RustinoCommand::Close),
            Phase::Running(proxy) => {
                let proxy = proxy.clone();
                drop(setup);
                let _ = proxy.send_event(RustinoCommand::Close);
            }
            Phase::Created | Phase::Exited => {}
        }
    }

    /// Lets go of the instance: returns true when the caller must free it.
    pub fn release(&self) -> bool {
        self.holders.fetch_sub(1, Ordering::AcqRel) == 1
    }

    fn setup(&self) -> MutexGuard<'_, Setup> {
        self.setup.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the configuration before the window runs. Returns false once it started.
    pub fn configure(&self, store: impl FnOnce(&mut Setup)) -> bool {
        let mut setup = self.setup();
        let created = matches!(setup.phase, Phase::Created);
        if created {
            store(&mut setup);
        }
        created
    }

    /// A setting of the window: before it runs `store` changes the configuration, then `cmd`
    /// changes the window.
    pub fn set(&self, cmd: RustinoCommand, store: impl FnOnce(&mut Setup)) {
        let mut setup = self.setup();
        match &setup.phase {
            Phase::Created => store(&mut setup),
            Phase::Starting => setup.pending.push(cmd),
            Phase::Running(proxy) => {
                // Not under the lock: the send may wait for the event loop
                let proxy = proxy.clone();
                drop(setup);
                let _ = proxy.send_event(cmd);
            }
            Phase::Exited => {}
        }
    }

    /// Sends a command to the window; before it runs, the window gets it once it exists.
    /// Returns false once the window closed.
    pub fn send_command(&self, cmd: RustinoCommand) -> bool {
        let mut setup = self.setup();
        match &setup.phase {
            Phase::Created | Phase::Starting => {
                setup.pending.push(cmd);
                true
            }
            Phase::Running(proxy) => {
                let proxy = proxy.clone();
                drop(setup);
                proxy.send_event(cmd).is_ok()
            }
            Phase::Exited => false,
        }
    }

    /// Sends a command to the running window only.
    pub fn send_to_running(&self, cmd: RustinoCommand) -> bool {
        let proxy = match &self.setup().phase {
            Phase::Running(proxy) => proxy.clone(),
            _ => return false,
        };
        proxy.send_event(cmd).is_ok()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.setup().phase, Phase::Running(_))
    }

    /// Gives the monitors to the host's handler, from the event loop, while the window runs: the
    /// host compares them to tell when a monitor is added or removed, which tao doesn't report.
    pub fn check_monitors(&self) {
        let callbacks = self.setup().callbacks;
        let Some(cb) = callbacks.on_monitors else {
            return;
        };
        // The context crosses as an address: the task runs on the event loop thread
        let context = callbacks.context as usize;
        let state = Arc::clone(&self.state);
        self.send_to_running(RustinoCommand::Invoke(crate::invoke::Task::new(move |window, _| {
            update_monitor_cache(window, &state);
            if let Ok(monitors) = CString::new(state.load_monitors()) {
                unsafe { cb(context as *mut std::ffi::c_void, monitors.as_ptr()) };
            }
        })));
    }

    /// Takes the configuration for `run`: a window runs once. The caller of `run` must `release`
    /// the instance afterwards.
    pub fn start(&self) -> Result<Started, String> {
        let mut setup = self.setup();
        match setup.phase {
            Phase::Created => {}
            Phase::Exited => return Err("The window already ran: create a new one.".into()),
            _ => return Err("The window is already running.".into()),
        }
        setup.phase = Phase::Starting;
        // Released by the caller of `run`
        self.holders.fetch_add(1, Ordering::AcqRel);
        Ok(Started {
            config: std::mem::take(&mut setup.config),
            callbacks: setup.callbacks,
            ext: std::mem::take(&mut setup.ext),
            webview_ext: std::mem::take(&mut setup.webview_ext),
        })
    }

    /// The window exists: the commands sent until now come first, in order.
    fn set_running(&self, proxy: CommandProxy) {
        let mut setup = self.setup();
        for cmd in setup.pending.drain(..) {
            let _ = proxy.send_event(cmd);
        }
        setup.phase = Phase::Running(proxy);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn settings_go_to_the_configuration_until_the_window_starts() {
        use super::{Phase, RustinoWindow};
        use crate::commands::RustinoCommand;
        let window = RustinoWindow::new(crate::config::WindowConfig::default());
        assert!(window.configure(|s| s.config.user_agent = Some("agent".into())));
        window.set(RustinoCommand::SetTitle("a".into()), |s| s.config.title = "a".into());
        assert!(window.send_command(RustinoCommand::EvaluateScript("1".into())), "queued");
        assert!(!window.is_running());

        let started = window.start().unwrap();
        assert_eq!(started.config.user_agent.as_deref(), Some("agent"));
        assert_eq!(started.config.title, "a");
        assert!(!window.configure(|s| s.config.user_agent = None), "the window started");
        window.set(RustinoCommand::SetTitle("b".into()), |s| s.config.title = "b".into());
        let setup = window.setup();
        assert!(matches!(setup.phase, Phase::Starting));
        assert!(matches!(
            setup.pending.as_slice(),
            [RustinoCommand::EvaluateScript(_), RustinoCommand::SetTitle(title)] if title == "b"
        ));
        assert_eq!(setup.config.title, "", "taken by run");
        drop(setup);
        assert!(window.start().is_err(), "a window runs once");
    }

    #[test]
    fn destroying_a_running_window_closes_it_and_waits_for_run() {
        use super::RustinoWindow;
        use crate::commands::RustinoCommand;
        let window = RustinoWindow::new(crate::config::WindowConfig::default());
        let _started = window.start().unwrap();
        window.close_for_destroy();
        assert!(!window.release(), "run still uses it");
        assert!(matches!(window.setup().pending.last(), Some(RustinoCommand::Close)));
        assert!(window.release(), "run is done: free it");

        let idle = RustinoWindow::new(crate::config::WindowConfig::default());
        idle.close_for_destroy();
        assert!(idle.release(), "never ran");
    }
}
