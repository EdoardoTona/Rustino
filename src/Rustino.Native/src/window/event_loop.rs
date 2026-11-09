//! The event loop of the app. The first window runs it on its thread; the windows started
//! while it runs join it, and close with the first one.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once, mpsc};

use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::run_return::EventLoopExtRunReturn;
#[cfg(target_os = "windows")]
use tao::platform::windows::EventLoopBuilderExtWindows;

use crate::commands::RustinoCommand;
use crate::state::SharedState;

use super::runtime::WindowRuntime;
use super::{Phase, RustinoWindow, Started};

/// What the event loop of the app receives.
pub enum LoopEvent {
    /// A command for the window with this key
    Command(usize, RustinoCommand),
    /// A window started while the event loop runs
    Host(Box<HostRequest>),
}

/// The only user event of tao's event loop: the events are waiting in the `LoopProxy` queue.
pub struct Wake;

/// Sends events to the event loop of the app. They wait in a queue, behind one wake-up message:
/// tao posts a message per event, and Windows drops the messages beyond the quota of a thread
/// (10,000), so commands sent faster than the loop handles them would get lost, `Close` included.
/// The queue is bounded too: otherwise threads sending without pause would starve the commands
/// sent after theirs (e.g. `Close`), behind an ever longer queue.
#[derive(Clone)]
pub struct LoopProxy {
    proxy: EventLoopProxy<Wake>,
    queue: Arc<(Mutex<LoopQueue>, Condvar)>,
}

/// Beyond this many events, senders wait for the loop (except the loop thread itself)
const QUEUE_LIMIT: usize = 10_000;
/// A sender waits at most this long, then queues anyway: it may hold a lock the loop is waiting for
const QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
/// Events handled per wake-up, so that the loop also handles the system's events in between
const BATCH: usize = 1_000;

#[derive(Default)]
struct LoopQueue {
    events: VecDeque<LoopEvent>,
    /// A wake-up message is on its way: the loop will take the events queued until then
    wake_pending: bool,
    /// The loop ended: nobody takes the events anymore
    closed: bool,
}

impl LoopProxy {
    fn new(proxy: EventLoopProxy<Wake>) -> Self {
        Self {
            proxy,
            queue: Default::default(),
        }
    }

    fn queue(&self) -> MutexGuard<'_, LoopQueue> {
        self.queue.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Fails, giving the event back, once the event loop ended.
    fn send(&self, event: LoopEvent) -> Result<(), LoopEvent> {
        let mut queue = self.queue();
        if queue.events.len() >= QUEUE_LIMIT && !crate::invoke::runs_event_loop() {
            queue = self
                .queue
                .1
                .wait_timeout_while(queue, QUEUE_WAIT, |q| !q.closed && q.events.len() >= QUEUE_LIMIT)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if queue.closed {
            return Err(event);
        }
        if !queue.wake_pending {
            if self.proxy.send_event(Wake).is_err() {
                return Err(event);
            }
            queue.wake_pending = true;
        }
        queue.events.push_back(event);
        Ok(())
    }

    /// The next events for the loop, on a wake-up. Posts another wake-up while events remain;
    /// otherwise the next `send` posts one.
    fn take(&self) -> Vec<LoopEvent> {
        let mut queue = self.queue();
        let count = queue.events.len().min(BATCH);
        let batch: Vec<_> = queue.events.drain(..count).collect();
        queue.wake_pending = !queue.events.is_empty() && self.proxy.send_event(Wake).is_ok();
        drop(queue);
        self.queue.1.notify_all();
        batch
    }

    /// The loop ended: the events still queued are dropped (e.g. the requests of windows that
    /// wanted to join, whose `run` then fails).
    fn close(&self) {
        let events = {
            let mut queue = self.queue();
            queue.closed = true;
            std::mem::take(&mut queue.events)
        };
        self.queue.1.notify_all();
        drop(events);
    }
}

/// Sends commands to one window of the event loop.
#[derive(Clone)]
pub struct CommandProxy {
    pub(super) key: usize,
    pub(super) proxy: LoopProxy,
}

impl CommandProxy {
    /// Fails once the event loop ended.
    pub fn send_event(&self, cmd: RustinoCommand) -> Result<(), ()> {
        self.proxy.send(LoopEvent::Command(self.key, cmd)).map_err(|_| ())
    }
}

/// A window to build in the event loop of the app. `reply` gets the outcome, then disconnects
/// once the window closes.
pub struct HostRequest {
    pub(super) key: usize,
    pub(super) state: Arc<SharedState>,
    pub(super) started: Started,
    pub(super) reply: mpsc::Sender<Result<(), String>>,
}

// The host callbacks and their context are only used on the event loop thread, as for the first
// window
unsafe impl Send for HostRequest {}

/// The event loop of the app: the windows started while it runs join it.
enum AppLoopState {
    Idle,
    /// The first window is being built on this thread: the other windows wait for its event loop
    Starting(std::thread::ThreadId),
    Running(LoopProxy),
}

static APP_LOOP: Mutex<AppLoopState> = Mutex::new(AppLoopState::Idle);
static APP_LOOP_CHANGED: Condvar = Condvar::new();

fn app_loop() -> MutexGuard<'static, AppLoopState> {
    APP_LOOP.lock().unwrap_or_else(|e| e.into_inner())
}

/// The event loop of the app, from the thread that starts it: back to idle when dropped, also
/// when the first window fails.
struct AppLoop;

impl AppLoop {
    /// Returns the event loop to join, or `AppLoop` when this thread starts it.
    fn join_or_start() -> Result<Result<LoopProxy, AppLoop>, String> {
        let this_thread = std::thread::current().id();
        let mut state = app_loop();
        loop {
            match &*state {
                AppLoopState::Running(proxy) => return Ok(Ok(proxy.clone())),
                AppLoopState::Idle => {
                    *state = AppLoopState::Starting(this_thread);
                    return Ok(Err(AppLoop));
                }
                // E.g. from a callback while the first window is built: waiting would never end
                AppLoopState::Starting(thread) if *thread == this_thread => {
                    return Err("The event loop of the app is starting on this thread: start the other windows from another thread.".into());
                }
                AppLoopState::Starting(_) => {
                    state = APP_LOOP_CHANGED.wait(state).unwrap_or_else(|e| e.into_inner());
                }
            }
        }
    }

    fn set_running(&self, proxy: LoopProxy) {
        *app_loop() = AppLoopState::Running(proxy);
        APP_LOOP_CHANGED.notify_all();
    }
}

impl Drop for AppLoop {
    fn drop(&mut self) {
        *app_loop() = AppLoopState::Idle;
        APP_LOOP_CHANGED.notify_all();
    }
}

impl RustinoWindow {
    /// Runs the window until it closes. The first window runs the event loop of the app on this
    /// thread (on macOS, the main thread), until it closes with the windows that joined it. The
    /// windows started while it runs join it and wait for their closing here. Fails when the
    /// window or the webview can't be created (e.g. without the WebView2 Runtime).
    pub fn run(&self, started: Started) -> Result<(), String> {
        // Whatever happens, the window no longer runs afterwards
        struct Exit<'a>(&'a RustinoWindow);
        impl Drop for Exit<'_> {
            fn drop(&mut self) {
                let mut setup = self.0.setup();
                setup.phase = Phase::Exited;
                let pending = std::mem::take(&mut setup.pending);
                drop(setup);
                drop(pending);
            }
        }
        let _exit = Exit(self);

        let app_loop = match AppLoop::join_or_start()? {
            Ok(proxy) => return self.join(started, proxy),
            // Dropped last, also on failure
            Err(app_loop) => app_loop,
        };

        let mut builder = EventLoopBuilder::<Wake>::with_user_event();
        #[cfg(target_os = "windows")]
        builder
            .with_any_thread(true)
            .with_msg_hook(|msg| crate::menu::accelerators::translate(unsafe { &*msg.cast() }));
        let mut event_loop = builder.build();
        let proxy = LoopProxy::new(event_loop.create_proxy());

        let main = WindowRuntime::new(self.key(), &self.state, started, &event_loop, &proxy, true)?;
        app_loop.set_running(proxy.clone());
        self.set_running(main.proxy.clone());

        // The first window runs the app: its closing closes the others
        let mut windows = vec![main];
        let loop_proxy = proxy.clone();
        event_loop.run_return(move |event, target, control_flow| {
            if *control_flow != ControlFlow::Exit {
                *control_flow = ControlFlow::Wait;
            }

            let (index, event) = match event {
                Event::UserEvent(Wake) => {
                    for event in proxy.take() {
                        // The app closes: the other windows close with it
                        if *control_flow == ControlFlow::Exit {
                            break;
                        }
                        match event {
                            LoopEvent::Host(request) => {
                                windows.extend(WindowRuntime::host(*request, target, &proxy));
                            }
                            LoopEvent::Command(key, cmd) => {
                                // The commands of a closed window are dropped
                                if let Some(index) = windows.iter().position(|w| w.key == key) {
                                    deliver(&mut windows, index, Event::UserEvent(cmd), target, control_flow);
                                }
                            }
                        }
                    }
                    return;
                }
                Event::MainEventsCleared => {
                    for window in &mut windows {
                        window.handle(Event::MainEventsCleared, target);
                    }
                    return;
                }
                Event::LoopDestroyed => {
                    proxy.close();
                    for window in &windows {
                        window.closed();
                    }
                    return;
                }
                event => {
                    let window_id = match &event {
                        Event::WindowEvent { window_id, .. } | Event::RedrawRequested(window_id) => {
                            Some(*window_id)
                        }
                        _ => None,
                    };
                    // The events of the app (opened URLs, Dock click, ...) go to the first window
                    let index = match window_id {
                        Some(id) => match windows.iter().position(|w| w.window.id() == id) {
                            Some(index) => index,
                            None => return,
                        },
                        None => 0,
                    };
                    let Ok(event) = event.map_nonuser_event() else {
                        return;
                    };
                    (index, event)
                }
            };
            deliver(&mut windows, index, event, target, control_flow);
        });
        loop_proxy.close();
        Ok(())
    }

    /// Runs the window in the event loop of the app, which runs on another thread, and waits for
    /// its closing.
    fn join(&self, started: Started, app_loop: LoopProxy) -> Result<(), String> {
        // Waiting here would block the event loop
        if crate::invoke::runs_event_loop() {
            return Err("The event loop of the app runs on this thread: start the other windows from another thread.".into());
        }
        let (reply, replies) = mpsc::channel();
        let request = HostRequest {
            key: self.key(),
            state: Arc::clone(&self.state),
            started,
            reply,
        };
        let closed = || "The app closed before the window could open.".to_string();
        app_loop
            .send(LoopEvent::Host(Box::new(request)))
            .map_err(|_| closed())?;
        replies.recv().map_err(|_| closed())??;
        self.set_running(CommandProxy {
            key: self.key(),
            proxy: app_loop,
        });
        // Disconnects once the window closes
        let _ = replies.recv();
        Ok(())
    }
}

/// The running windows, which receive the menu and tray events.
static EVENT_LOOPS: Mutex<Vec<(u64, CommandProxy)>> = Mutex::new(Vec::new());

/// The window stays registered until the returned value is dropped.
pub(super) fn register_event_loop(
    proxy: CommandProxy,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] window: &tao::window::Window,
) -> EventLoopRegistration {
    static NEXT_KEY: AtomicU64 = AtomicU64::new(0);
    static INSTALL_HANDLERS: Once = Once::new();
    // muda and tray-icon keep the first event handler for the whole process: it forwards the
    // events to every running window, which ignores the menu items and tray icons of the others
    INSTALL_HANDLERS.call_once(|| {
        muda::MenuEvent::set_event_handler(Some(|event: muda::MenuEvent| {
            send_to_event_loops(|| RustinoCommand::MenuEventFired(event.id.clone()));
        }));
        tray_icon::TrayIconEvent::set_event_handler(Some(|event| {
            if let tray_icon::TrayIconEvent::Click {
                id,
                position,
                button,
                button_state,
                ..
            } = event
            {
                send_to_event_loops(|| RustinoCommand::TrayIconButton {
                    id: id.clone(),
                    button,
                    pressed: button_state == tray_icon::MouseButtonState::Down,
                    x: position.x as i32,
                    y: position.y as i32,
                });
            }
        }));
    });

    let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut loops) = EVENT_LOOPS.lock() {
        loops.push((key, proxy));
    }
    EventLoopRegistration {
        key,
        #[cfg(target_os = "windows")]
        hwnd: {
            use tao::platform::windows::WindowExtWindows;
            window.hwnd()
        },
    }
}

pub(super) struct EventLoopRegistration {
    key: u64,
    /// The menu accelerators of the window go with it
    #[cfg(target_os = "windows")]
    hwnd: isize,
}

impl Drop for EventLoopRegistration {
    fn drop(&mut self) {
        if let Ok(mut loops) = EVENT_LOOPS.lock() {
            loops.retain(|(k, _)| *k != self.key);
        }
        #[cfg(target_os = "windows")]
        crate::menu::accelerators::set_menu(self.hwnd, None, 0);
    }
}

/// Gives an event to the window at `index`, which may close: the first window ends the event loop.
fn deliver(
    windows: &mut Vec<WindowRuntime>,
    index: usize,
    event: Event<'_, RustinoCommand>,
    target: &EventLoopWindowTarget<Wake>,
    control_flow: &mut ControlFlow,
) {
    if windows[index].handle(event, target) {
        if index == 0 {
            *control_flow = ControlFlow::Exit;
        } else {
            close_window(windows, windows[index].key);
        }
    }
}

/// Closes the window `key` after its child windows, which close with it.
fn close_window(windows: &mut Vec<WindowRuntime>, key: usize) {
    while let Some(child) = windows.iter().find(|w| w.config.parent == Some(key)).map(|w| w.key) {
        close_window(windows, child);
    }
    if let Some(index) = windows.iter().position(|w| w.key == key) {
        windows.remove(index).closed();
    }
}

pub(crate) fn send_to_event_loops(command: impl Fn() -> RustinoCommand) {
    if let Ok(loops) = EVENT_LOOPS.lock() {
        for (_, proxy) in loops.iter() {
            let _ = proxy.send_event(command());
        }
    }
}

#[cfg(test)]
mod tests {
    /// The windows started while the app runs join its event loop (on macOS the only one there
    /// can be), and close with its first window. Opens hidden windows: needs WebView2.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_started_while_the_app_runs_join_its_event_loop() {
        use super::RustinoWindow;
        use crate::commands::RustinoCommand;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;
        use std::time::{Duration, Instant};

        // The host's handle: the callbacks run on the event loop thread
        #[derive(Clone, Copy)]
        struct Handle(&'static RustinoWindow);
        unsafe impl Send for Handle {}

        unsafe extern "C" fn on_closed(context: *mut std::ffi::c_void) {
            unsafe { &*(context as *const AtomicUsize) }.fetch_add(1, Ordering::SeqCst);
        }

        fn new_window(closed: &'static AtomicUsize) -> Handle {
            let mut config = crate::config::WindowConfig::default();
            config.visible = false;
            config.start_html = Some("<p>Rustino</p>".into());
            let window = Box::leak(Box::new(RustinoWindow::new(config)));
            window.configure(|s| {
                s.callbacks.context = closed as *const AtomicUsize as *mut _;
                s.callbacks.on_closed = Some(on_closed);
            });
            Handle(window)
        }

        fn run(window: Handle) -> thread::JoinHandle<Result<(), String>> {
            thread::spawn(move || {
                let window = window;
                let started = window.0.start()?;
                window.0.run(started)
            })
        }

        fn wait_until(what: &str, condition: impl Fn() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !condition() {
                assert!(Instant::now() < deadline, "timed out waiting until {what}");
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn loop_thread(window: Handle) -> thread::ThreadId {
            window.0.invoke(|_, _| thread::current().id()).expect("the window runs")
        }

        let closed: [&'static AtomicUsize; 3] = std::array::from_fn(|_| &*Box::leak(Box::new(AtomicUsize::new(0))));
        let main = new_window(closed[0]);
        let main_run = run(main);
        wait_until("the main window runs", || main.0.is_running());

        // A window joins the event loop and closes alone
        let second = new_window(closed[1]);
        let second_run = run(second);
        wait_until("the second window runs", || second.0.is_running());
        assert_eq!(loop_thread(second), loop_thread(main), "one event loop");
        assert!(second.0.send_command(RustinoCommand::Close));
        assert_eq!(second_run.join().unwrap(), Ok(()));
        assert_eq!(closed[1].load(Ordering::SeqCst), 1, "closed once");
        assert!(main.0.is_running(), "the main window stays");
        assert_eq!(closed[0].load(Ordering::SeqCst), 0);

        // The event loop thread can't wait for another window
        let blocked = Handle(Box::leak(Box::new(RustinoWindow::new(Default::default()))));
        let error = main
            .0
            .invoke(move |_, _| {
                let blocked = blocked;
                let started = blocked.0.start()?;
                blocked.0.run(started)
            })
            .unwrap();
        assert!(error.unwrap_err().contains("another thread"));

        // The main window closes the windows that joined it
        let third = new_window(closed[2]);
        let third_run = run(third);
        wait_until("the third window runs", || third.0.is_running());
        assert!(main.0.send_command(RustinoCommand::Close));
        assert_eq!(main_run.join().unwrap(), Ok(()));
        assert_eq!(third_run.join().unwrap(), Ok(()));
        assert_eq!(closed[0].load(Ordering::SeqCst), 1);
        assert_eq!(closed[2].load(Ordering::SeqCst), 1);
        assert!(!third.0.is_running());
        assert!(matches!(*super::app_loop(), super::AppLoopState::Idle), "the app closed");

        // Windows started together share one event loop: the others wait for the first one
        let unused = Box::leak(Box::new(AtomicUsize::new(0)));
        let together: Vec<Handle> = (0..3).map(|_| new_window(unused)).collect();
        let barrier = Arc::new(std::sync::Barrier::new(together.len()));
        let runs: Vec<_> = together
            .iter()
            .map(|&window| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let window = window;
                    let started = window.0.start()?;
                    barrier.wait();
                    window.0.run(started)
                })
            })
            .collect();
        for window in &together {
            wait_until("the windows started together run", || window.0.is_running());
        }
        let first = loop_thread(together[0]);
        assert!(together.iter().all(|&w| loop_thread(w) == first), "one event loop");
        for window in &together {
            window.0.send_command(RustinoCommand::Close);
        }
        for run in runs {
            assert_eq!(run.join().unwrap(), Ok(()));
        }
        assert!(matches!(*super::app_loop(), super::AppLoopState::Idle));
    }
}
