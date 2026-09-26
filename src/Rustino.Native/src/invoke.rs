//! Runs code on the event loop thread, which owns the window and the webview: the webview APIs
//! work only there, and dialogs need the window as their parent.
//!
//! Called from that thread (e.g. from a callback of the host), the code runs right away: waiting
//! for the event loop there would deadlock.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use tao::window::Window;
use wry::WebView;

use crate::commands::RustinoCommand;
use crate::window::RustinoWindow;

type TaskFn = dyn FnOnce(&Window, &WebView) + Send;

/// Code sent to the event loop with `RustinoCommand::Invoke`.
pub struct Task(Box<TaskFn>);

impl Task {
    pub fn new(task: impl FnOnce(&Window, &WebView) + Send + 'static) -> Self {
        Self(Box::new(task))
    }

    pub fn run(self, window: &Window, webview: &WebView) {
        (self.0)(window, webview)
    }
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Task")
    }
}

struct Running {
    key: usize,
    // Dropped before the window
    webview: Rc<WebView>,
    window: Rc<Window>,
}

thread_local! {
    /// The windows whose event loop runs on this thread
    static RUNNING: RefCell<Vec<Running>> = const { RefCell::new(Vec::new()) };
    /// Host callbacks running within a webview event handler
    static WEBVIEW_EVENTS: Cell<u32> = const { Cell::new(0) };
}

/// Runs a host callback called by a webview event handler (IPC message, navigation, ...).
pub fn webview_event<R>(callback: impl FnOnce() -> R) -> R {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            WEBVIEW_EVENTS.with(|n| n.set(n.get() - 1));
        }
    }
    WEBVIEW_EVENTS.with(|n| n.set(n.get() + 1));
    let _leave = Leave;
    callback()
}

/// Whether this thread can wait for an asynchronous webview operation. WebView2 completes them
/// only after the event handler that is running returns: waiting within one never ends.
pub fn can_wait_for_webview() -> bool {
    !cfg!(target_os = "windows") || WEBVIEW_EVENTS.with(|n| n.get() == 0)
}

/// Keeps the window reachable from its event loop thread until dropped.
pub struct RunningGuard(usize);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        let removed = RUNNING.with(|running| {
            let mut running = running.borrow_mut();
            running.iter().position(|r| r.key == self.0).map(|i| running.remove(i))
        });
        drop(removed);
    }
}

impl RustinoWindow {
    fn key(&self) -> usize {
        self as *const Self as usize
    }

    /// Called by `run` on the event loop thread once the webview exists.
    pub(crate) fn register_running(&self, window: &Rc<Window>, webview: &Rc<WebView>) -> RunningGuard {
        RUNNING.with(|running| {
            running.borrow_mut().push(Running {
                key: self.key(),
                webview: Rc::clone(webview),
                window: Rc::clone(window),
            })
        });
        RunningGuard(self.key())
    }

    /// The window and the webview, when called from the event loop thread.
    fn running_here(&self) -> Option<(Rc<Window>, Rc<WebView>)> {
        RUNNING.with(|running| {
            let running = running.borrow();
            let r = running.iter().find(|r| r.key == self.key())?;
            Some((Rc::clone(&r.window), Rc::clone(&r.webview)))
        })
    }

    pub fn is_running(&self) -> bool {
        self.proxy.read().is_ok_and(|proxy| proxy.is_some())
    }

    /// Runs `task` on the event loop thread and returns its result, waiting for it when called
    /// from another thread. `None` when the window doesn't run or closes first.
    pub fn invoke<R: Send + 'static>(
        &self,
        task: impl FnOnce(&Window, &WebView) -> R + Send + 'static,
    ) -> Option<R> {
        if let Some((window, webview)) = self.running_here() {
            return Some(task(&window, &webview));
        }
        let (tx, rx) = mpsc::channel();
        let sent = self.send_command(RustinoCommand::Invoke(Task::new(move |window, webview| {
            let _ = tx.send(task(window, webview));
        })));
        // A closing window drops the task, and with it the sender
        if sent { rx.recv().ok() } else { None }
    }

    /// Runs `task` on the event loop thread without waiting for it. Returns false when the
    /// window doesn't run.
    pub fn post(&self, task: impl FnOnce(&Window, &WebView) + Send + 'static) -> bool {
        match self.running_here() {
            Some((window, webview)) => {
                task(&window, &webview);
                true
            }
            None => self.send_command(RustinoCommand::Invoke(Task::new(task))),
        }
    }
}
