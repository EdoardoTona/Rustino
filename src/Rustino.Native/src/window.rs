use std::borrow::Cow;
use std::ffi::CString;
use std::rc::Rc;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once, mpsc};

use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::WindowBuilder;
#[cfg(target_os = "windows")]
use tao::platform::windows::EventLoopBuilderExtWindows;
use wry::WebViewBuilder;

use crate::callbacks::RustinoCallbacks;
use crate::commands::RustinoCommand;
use crate::config::WindowConfig;
use crate::icon;
use crate::menu;
use crate::state::SharedState;

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
    key: usize,
    proxy: LoopProxy,
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
    key: usize,
    state: Arc<SharedState>,
    started: Started,
    reply: mpsc::Sender<Result<(), String>>,
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
            .with_msg_hook(|msg| crate::accelerators::translate(unsafe { &*msg.cast() }));
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

    /// The window exists: the commands sent until now come first, in order.
    fn set_running(&self, proxy: CommandProxy) {
        let mut setup = self.setup();
        for cmd in setup.pending.drain(..) {
            let _ = proxy.send_event(cmd);
        }
        setup.phase = Phase::Running(proxy);
    }
}

/// A window of the event loop, with what its events need.
struct WindowRuntime {
    key: usize,
    proxy: CommandProxy,
    callbacks: RustinoCallbacks,
    config: WindowConfig,
    state: Arc<SharedState>,
    ext: crate::window_ext::WindowExtRuntime,
    menu_items: menu::MenuItems,
    current_menu: Option<muda::Menu>,
    tray: Option<Tray>,
    _menu_events: EventLoopRegistration,
    /// Keeps the window and the webview reachable from `invoke`
    _running: crate::invoke::RunningGuard,
    // Dropped before the window
    webview: Rc<wry::WebView>,
    window: Rc<tao::window::Window>,
    _web_context: Option<wry::WebContext>,
    /// Windows that joined the event loop: disconnects once the window closes, which ends its `run`
    closed_reply: Option<mpsc::Sender<Result<(), String>>>,
}

impl WindowRuntime {
    /// Builds a window that joins the running event loop, and tells its `run` the outcome.
    fn host(
        request: HostRequest,
        target: &EventLoopWindowTarget<Wake>,
        proxy: &LoopProxy,
    ) -> Option<Self> {
        let HostRequest {
            key,
            state,
            started,
            reply,
        } = request;
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::new(key, &state, started, target, proxy, false)
        }))
        .unwrap_or_else(|panic| Err(crate::panic_message(panic.as_ref())));
        match built {
            Ok(mut window) => {
                // The event loop started before the window
                window.handle(Event::NewEvents(StartCause::Init), target);
                let _ = reply.send(Ok(()));
                window.closed_reply = Some(reply);
                Some(window)
            }
            Err(message) => {
                let _ = reply.send(Err(message));
                None
            }
        }
    }

    /// Builds the window and its webview. `main`: the window that runs the event loop.
    fn new(
        key: usize,
        state: &Arc<SharedState>,
        started: Started,
        target: &EventLoopWindowTarget<Wake>,
        proxy: &LoopProxy,
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] main: bool,
    ) -> Result<Self, String> {
        let Started {
            config,
            callbacks,
            ext,
            webview_ext,
        } = started;

        warn_unsupported_settings(&config);
        let proxy = CommandProxy {
            key,
            proxy: proxy.clone(),
        };

        // --- Build window ---
        let mut window_builder = WindowBuilder::new()
            .with_title(&config.title)
            .with_resizable(config.resizable)
            .with_always_on_top(config.topmost)
            .with_decorations(config.decorations)
            .with_visible(config.visible)
            .with_maximized(config.maximized);

        if config.transparent {
            window_builder = window_builder.with_transparent(true);
        }

        if !config.use_os_default_size {
            window_builder =
                window_builder.with_inner_size(PhysicalSize::new(config.width, config.height));
        }

        if let Some((w, h)) = config.min_size {
            window_builder = window_builder.with_min_inner_size(PhysicalSize::new(w, h));
        }

        if let Some((w, h)) = config.max_size {
            window_builder = window_builder.with_max_inner_size(PhysicalSize::new(w, h));
        }

        if let Some(ref icon_path) = config.icon_file
            && let Some(ico) = icon::load_icon(icon_path)
        {
            window_builder = window_builder.with_window_icon(Some(ico));
        }

        if let Some(color) = config.background_color {
            window_builder = window_builder.with_background_color(color);
        }

        window_builder = ext.configure_window(window_builder);

        let window = window_builder
            .build(target)
            .map_err(|e| format!("Failed to create the window: {e}"))?;

        if config.center {
            center_window(&window);
        }

        if let Some((x, y)) = config.position {
            window.set_outer_position(PhysicalPosition::new(x, y));
        }

        if config.fullscreen {
            window.set_fullscreen(Some(tao::window::Fullscreen::Borderless(None)));
        }

        // --- Build webview ---
        let mut web_context = config
            .user_data_folder
            .as_ref()
            .map(|p| wry::WebContext::new(Some(std::path::PathBuf::from(p))));

        let mut webview_builder = match web_context {
            Some(ref mut ctx) => WebViewBuilder::new_with_web_context(ctx),
            None => WebViewBuilder::new(),
        };

        if config.devtools_enabled {
            webview_builder = webview_builder.with_devtools(true);
        }

        if config.clipboard_enabled {
            webview_builder = webview_builder.with_clipboard(true);
        }

        if config.transparent {
            webview_builder = webview_builder.with_transparent(true);
        }

        if let Some(color) = config.background_color {
            webview_builder = webview_builder.with_background_color(color);
        }

        if let Some(ref ua) = config.user_agent {
            webview_builder = webview_builder.with_user_agent(ua);
        }

        webview_builder = webview_builder.with_autoplay(config.media_autoplay);

        #[cfg(target_os = "windows")]
        if let Some(args) = webview2_browser_args(&config) {
            use wry::WebViewBuilderExtWindows;
            webview_builder = webview_builder.with_additional_browser_args(args);
        }

        if config.zoom_hotkeys {
            webview_builder = webview_builder.with_hotkeys_zoom(true);
        }

        webview_builder = ext.configure_webview(webview_builder);
        let ipc_filter = ext.ipc_filter(proxy.clone());
        let print_filter = crate::webview_ext::PrintFilter::new(proxy.clone());
        webview_builder = webview_ext.configure(
            webview_builder,
            callbacks.context,
            state,
            proxy.clone(),
            config.logger(),
        );

        for script in &config.initialization_scripts {
            webview_builder = webview_builder.with_initialization_script(script);
        }

        // IPC handler: JS → Rust, with the URL of the page that sent the message
        let ctx = callbacks.context;
        if let Some(cb) = callbacks.on_web_message {
            #[cfg(target_os = "windows")]
            let schemes = config.custom_schemes.clone();
            webview_builder = webview_builder.with_ipc_handler(move |req: wry::http::Request<String>| {
                if print_filter.handle(req.body()) || ipc_filter.handle(req.body()) {
                    return;
                }
                let source = req.uri().to_string();
                #[cfg(target_os = "windows")]
                let source = revert_custom_scheme_workaround(&source, &schemes);
                if let (Ok(message), Ok(source)) = (CString::new(req.into_body()), CString::new(source)) {
                    crate::invoke::webview_event(|| unsafe { cb(ctx, message.as_ptr(), source.as_ptr()) });
                }
            });
        } else {
            webview_builder = webview_builder.with_ipc_handler(move |req: wry::http::Request<String>| {
                let _ = print_filter.handle(req.body()) || ipc_filter.handle(req.body());
            });
        }

        // Navigation handler
        if let Some(cb) = callbacks.on_navigation {
            webview_builder = webview_builder.with_navigation_handler(move |url| {
                match CString::new(url) {
                    Ok(cstr) => crate::invoke::webview_event(|| unsafe { cb(ctx, cstr.as_ptr()) == 0 }),
                    Err(_) => true,
                }
            });

            webview_builder = webview_builder.with_new_window_req_handler(move |url, _features| {
                handle_new_window_req(url, ctx, cb)
            });
        }

        // Custom scheme handlers: webview request → host response
        if let Some(cb) = callbacks.on_custom_scheme {
            for scheme in &config.custom_schemes {
                webview_builder = webview_builder.with_custom_protocol(scheme.clone(), move |_id, request| {
                    crate::invoke::webview_event(|| handle_custom_scheme(request.uri().to_string(), ctx, cb))
                });
            }
        }

        // Page load handler
        if let Some(cb) = callbacks.on_page_load {
            webview_builder =
                webview_builder.with_on_page_load_handler(move |event, url| {
                    let event_code = match event {
                        wry::PageLoadEvent::Started => 0,
                        wry::PageLoadEvent::Finished => 1,
                    };
                    if let Ok(cstr) = CString::new(url) {
                        crate::invoke::webview_event(|| unsafe { cb(ctx, event_code, cstr.as_ptr()) });
                    }
                });
        }

        if let Some(ref url) = config.start_url {
            webview_builder = webview_builder.with_url(url);
        } else if let Some(ref html) = config.start_html {
            webview_builder = webview_builder.with_html(html);
        }

        // On Linux the webview goes in tao's GTK box, next to the menu bar: `build` only
        // supports X11 and would draw over the menu bar
        #[cfg(target_os = "linux")]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            let vbox = window.default_vbox().ok_or("Failed to create the webview: the window has no GTK box")?;
            webview_builder.build_gtk(vbox)
        };
        #[cfg(not(target_os = "linux"))]
        let webview = webview_builder.build(&window);
        let webview = webview.map_err(|e| format!("Failed to create the webview: {e}"))?;

        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowExtWindows;
            crate::accelerators::attach_webview(&webview, window.hwnd());
            crate::webview_ext::allow_multiple_downloads(&webview);
        }

        #[cfg(target_os = "macos")]
        if config.devtools_enabled {
            crate::webview_ext::make_inspectable(&webview);
        }

        // Reachable from the callbacks, which run on this thread
        let (window, webview) = (Rc::new(window), Rc::new(webview));
        let running = crate::invoke::register_running(key, &window, &webview);

        // Initialize shared state from actual window
        let size = window.inner_size();
        let pos = window
            .outer_position()
            .unwrap_or(PhysicalPosition::new(0, 0));
        state.store_size(size.width, size.height);
        state.store_position(pos.x, pos.y);
        state
            .is_maximized
            .store(window.is_maximized(), Ordering::Release);
        state
            .is_fullscreen
            .store(config.fullscreen, Ordering::Release);
        state
            .is_visible
            .store(config.visible, Ordering::Release);

        update_monitor_cache(&window, state);
        let ext = ext.start(&window, state, config.decorations, callbacks.context, proxy.clone());

        // Menu and tray events reach this window through its event loop
        let menu_events = register_event_loop(proxy.clone(), &window);

        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut menu_items = menu::MenuItems::default();
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut current_menu: Option<muda::Menu> = None;
        // The menu bar of macOS belongs to the app: the other windows keep it
        #[cfg(target_os = "macos")]
        if main {
            let about = about_metadata(&config);
            if let Some(built) = menu::build_menu(DEFAULT_MACOS_MENU, &about) {
                set_menu_bar(built, &window, &config, &about, &mut current_menu, &mut menu_items);
            }
        }

        Ok(Self {
            key,
            proxy,
            callbacks,
            config,
            state: Arc::clone(state),
            ext,
            menu_items,
            current_menu,
            tray: None,
            _menu_events: menu_events,
            _running: running,
            webview,
            window,
            _web_context: web_context,
            closed_reply: None,
        })
    }

    /// Handles an event of this window. Returns true when the window closes.
    fn handle(&mut self, event: Event<'_, RustinoCommand>, target: &EventLoopWindowTarget<Wake>) -> bool {
        let Some(event) = self.ext.handle_event(event, target, &self.window, &self.webview) else {
            return false;
        };

        let (window, state, callbacks) = (&self.window, &self.state, self.callbacks);
        #[allow(clippy::collapsible_match)]
        match event {
            Event::UserEvent(cmd) => {
                return dispatch_command(
                    cmd,
                    window,
                    &self.webview,
                    state,
                    callbacks,
                    &mut self.menu_items,
                    &mut self.current_menu,
                    &mut self.config,
                    &mut self.tray,
                );
            }
            Event::WindowEvent {
                event: ref win_event, ..
            } => {
                // These flags can change from the native window manager without a
                // Rustino command (for example, the user minimizes the window).
                state
                    .is_minimized
                    .store(window.is_minimized(), Ordering::Release);
                state
                    .is_fullscreen
                    .store(window.fullscreen().is_some(), Ordering::Release);

                match win_event {
                    WindowEvent::CloseRequested => {
                        if let Some(cb) = callbacks.on_closing {
                            if unsafe { cb(callbacks.context) } != 0 {
                                return false;
                            }
                        }
                        return true;
                    }
                    WindowEvent::Resized(size) => {
                        state.store_size(size.width, size.height);
                        state
                            .is_maximized
                            .store(window.is_maximized(), Ordering::Release);
                        if let Some(cb) = callbacks.on_resized {
                            unsafe {
                                cb(callbacks.context, size.width as i32, size.height as i32)
                            };
                        }
                    }
                    WindowEvent::Moved(pos) => {
                        state.store_position(pos.x, pos.y);
                        update_monitor_cache(window, state);
                        if let Some(cb) = callbacks.on_moved {
                            unsafe { cb(callbacks.context, pos.x, pos.y) };
                        }
                    }
                    WindowEvent::Focused(focused) => {
                        state.is_focused.store(*focused, Ordering::Release);
                        if let Some(cb) = callbacks.on_focus_changed {
                            unsafe {
                                cb(callbacks.context, if *focused { 1 } else { 0 })
                            };
                        }
                    }
                    WindowEvent::KeyboardInput { .. } => {
                        // Forward keyboard events to the webview by not consuming them
                        // The webview's internal handler will process these events
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        false
    }

    /// Tells the host that the window closed.
    fn closed(&self) {
        if let Some(cb) = self.callbacks.on_closed {
            unsafe { cb(self.callbacks.context) };
        }
    }
}

/// The running windows, which receive the menu and tray events.
static EVENT_LOOPS: Mutex<Vec<(u64, CommandProxy)>> = Mutex::new(Vec::new());

/// The window stays registered until the returned value is dropped.
fn register_event_loop(
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

struct EventLoopRegistration {
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
        crate::accelerators::set_menu(self.hwnd, None, 0);
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
            windows.remove(index).closed();
        }
    }
}

fn send_to_event_loops(command: impl Fn() -> RustinoCommand) {
    if let Ok(loops) = EVENT_LOOPS.lock() {
        for (_, proxy) in loops.iter() {
            let _ = proxy.send_event(command());
        }
    }
}

/// Browser arguments of WebView2 for the settings it has no API for; `None` keeps wry's own.
/// They apply to this webview only (not through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, which
/// would reach every later window of the process). Windows sharing a user data folder need the
/// same arguments: WebView2 refuses to create the others.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn webview2_browser_args(config: &WindowConfig) -> Option<String> {
    let mut extra = Vec::new();
    if !config.web_security_enabled {
        extra.push("--disable-web-security");
    }
    if config.ignore_certificate_errors {
        extra.push("--ignore-certificate-errors");
    }
    if extra.is_empty() {
        return None;
    }
    // Custom arguments replace wry's defaults (wry 0.55): its mini menu and SmartScreen off,
    // autoplay without a user gesture
    let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if config.media_autoplay {
        args.push_str(" --autoplay-policy=no-user-gesture-required");
    }
    for arg in extra {
        args.push(' ');
        args.push_str(arg);
    }
    Some(args)
}

/// Settings that only WebView2 supports.
fn warn_unsupported_settings(#[allow(unused_variables)] config: &WindowConfig) {
    #[cfg(not(target_os = "windows"))]
    {
        if !config.web_security_enabled {
            config.logger().warning("[rustino] Warning: SetWebSecurityEnabled(false) is only supported on Windows (WebView2). Ignored on this platform.");
        }
        if config.ignore_certificate_errors {
            config.logger().warning("[rustino] Warning: SetIgnoreCertificateErrorsEnabled(true) is only supported on Windows (WebView2). Ignored on this platform.");
        }
    }
}

/// The host can't get the result of a queued command: tell its logger when one fails.
fn report_failure(config: &WindowConfig, operation: &str, result: wry::Result<()>) {
    if let Err(e) = result {
        config.logger().error(&format!("[rustino] {operation} failed: {e}"));
    }
}

fn center_window(window: &tao::window::Window) {
    if let Some(monitor) = window.current_monitor() {
        let monitor_size = monitor.size();
        let monitor_pos = monitor.position();
        let window_size = window.outer_size();
        let x = monitor_pos.x + ((monitor_size.width as i32 - window_size.width as i32) / 2);
        let y = monitor_pos.y + ((monitor_size.height as i32 - window_size.height as i32) / 2);
        window.set_outer_position(PhysicalPosition::new(x, y));
    }
}

fn update_monitor_cache(window: &tao::window::Window, state: &SharedState) {
    let primary = window.primary_monitor();
    let monitors: Vec<serde_json::Value> = window
        .available_monitors()
        .map(|m| {
            let pos = m.position();
            let size = m.size();
            let is_primary = primary
                .as_ref()
                .map_or(false, |p| p.name() == m.name() && p.position() == m.position());
            serde_json::json!({
                "name": m.name(),
                "x": pos.x,
                "y": pos.y,
                "width": size.width,
                "height": size.height,
                "scaleFactor": m.scale_factor(),
                "isPrimary": is_primary
            })
        })
        .collect();
    let monitors_json = serde_json::to_string(&monitors).unwrap_or_default();

    let current_json = if let Some(m) = window.current_monitor() {
        let pos = m.position();
        let size = m.size();
        let is_primary = primary
            .as_ref()
            .map_or(false, |p| p.name() == m.name() && p.position() == m.position());
        serde_json::to_string(&serde_json::json!({
            "name": m.name(),
            "x": pos.x,
            "y": pos.y,
            "width": size.width,
            "height": size.height,
            "scaleFactor": m.scale_factor(),
            "isPrimary": is_primary
        }))
        .unwrap_or_default()
    } else {
        String::new()
    };

    state.store_monitors(&monitors_json, &current_json);
}

fn dispatch_command(
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
            if let Some(ico) = load_tray_icon(&params.icon_path) {
                let mut builder = tray_icon::TrayIconBuilder::new()
                    .with_icon(ico)
                    .with_icon_as_template(params.icon_is_template)
                    .with_menu_on_left_click(params.menu_on_left_click);
                // The tooltip is the icon's accessible name: fall back to the window title
                let tooltip = params.tooltip.clone().unwrap_or_else(|| window.title());
                if !tooltip.is_empty() {
                    builder = builder.with_tooltip(tooltip);
                }
                if let Some(ref title) = params.title {
                    builder = builder.with_title(title);
                }
                let mut has_menu = false;
                if let Some(ref menu_json) = params.menu_json {
                    if let Some(built) = menu::build_menu(menu_json, &about_metadata(config)) {
                        has_menu = !built.menu.items().is_empty();
                        menu_items.replace(menu::MenuOwner::Tray, built.items);
                        builder = builder.with_menu(Box::new(built.menu));
                    }
                }
                *tray = builder.build().ok().map(|icon| Tray {
                    icon,
                    has_menu,
                    menu_on_left_click: params.menu_on_left_click,
                });
            }
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
        RustinoCommand::GetMonitors(tx) => {
            update_monitor_cache(window, state);
            let _ = tx.send(state.load_monitors());
        }
        RustinoCommand::GetCurrentMonitor(tx) => {
            update_monitor_cache(window, state);
            let _ = tx.send(state.load_current_monitor());
        }
        RustinoCommand::Close => return true,
    }
    false
}

// --- Menu platform helpers ---

/// Menu bar shown on macOS until `SetMenu`, after the standard application menu. Cmd+C/V/X/A/Z
/// reach the webview only through the Edit items.
#[cfg(any(target_os = "macos", test))]
const DEFAULT_MACOS_MENU: &str = r#"[
    {"type": "submenu", "label": "Edit", "items": [
        {"type": "predefined", "item": "undo"},
        {"type": "predefined", "item": "redo"},
        {"type": "separator"},
        {"type": "predefined", "item": "cut"},
        {"type": "predefined", "item": "copy"},
        {"type": "predefined", "item": "paste"},
        {"type": "predefined", "item": "select_all"}
    ]},
    {"type": "submenu", "label": "Window", "role": "window", "items": [
        {"type": "predefined", "item": "minimize"},
        {"type": "predefined", "item": "maximize"},
        {"type": "separator"},
        {"type": "predefined", "item": "bring_all_to_front"}
    ]}
]"#;

/// Replaces the window's menu bar (on macOS, the application's).
fn set_menu_bar(
    built: menu::BuiltMenu,
    window: &tao::window::Window,
    _config: &WindowConfig,
    _about: &muda::AboutMetadata,
    current_menu: &mut Option<muda::Menu>,
    menu_items: &mut menu::MenuItems,
) {
    if let Some(old) = current_menu.take() {
        remove_menu_from_window(&old, window);
    }
    // On macOS the first submenu of the menu bar becomes the application menu: without
    // AddAppMenu the standard one is prepended so the first custom submenu stays visible.
    #[cfg(target_os = "macos")]
    let app_menu = built
        .app_menu
        .or_else(|| Some(create_macos_app_menu(_config, _about)));
    #[cfg(not(target_os = "macos"))]
    let app_menu = built.app_menu;
    if let Some(app_menu) = app_menu {
        let _ = built.menu.prepend(&app_menu);
    }
    attach_menu_to_window(&built.menu, built.custom_edit_shortcuts, window);
    // After init_for_nsapp, as muda requires
    #[cfg(target_os = "macos")]
    {
        if let Some(window_menu) = &built.window_menu {
            window_menu.set_as_windows_menu_for_nsapp();
        }
        if let Some(help_menu) = &built.help_menu {
            help_menu.set_as_help_menu_for_nsapp();
        }
    }
    menu_items.replace(menu::MenuOwner::MenuBar, built.items);
    *current_menu = Some(built.menu);
}

/// Standard macOS application menu: About, Hide, Hide Others, Show All, Quit.
#[cfg(target_os = "macos")]
pub(crate) fn create_macos_app_menu(
    config: &crate::config::WindowConfig,
    about: &muda::AboutMetadata,
) -> muda::Submenu {
    let app_menu = muda::Submenu::new("App", true);

    // Without SetAboutName, muda and AppKit use the app name shown in the menu bar
    let name = config.about_name.as_deref();
    let label = |action: &str| name.map(|name| format!("{action} {name}"));
    let (about_label, hide, quit) = (label("About"), label("Hide"), label("Quit"));

    let _ = app_menu.append(&muda::PredefinedMenuItem::about(
        about_label.as_deref(),
        Some(about.clone()),
    ));
    let _ = app_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = app_menu.append(&muda::PredefinedMenuItem::hide(hide.as_deref()));
    let _ = app_menu.append(&muda::PredefinedMenuItem::hide_others(None));
    let _ = app_menu.append(&muda::PredefinedMenuItem::show_all(None));
    let _ = app_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = app_menu.append(&muda::PredefinedMenuItem::quit(quit.as_deref()));

    app_menu
}

/// Metadata for the predefined About item, from the SetAbout* values and the window icon.
fn about_metadata(config: &crate::config::WindowConfig) -> muda::AboutMetadata {
    let authors = (!config.about_authors.is_empty()).then(|| config.about_authors.clone());
    // macOS's About panel ignores comments, authors, website and license: show them as credits
    let credits = [
        config.about_comments.clone(),
        authors.as_ref().map(|a| a.join(", ")),
        config.about_website.clone(),
        config.about_license.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");

    muda::AboutMetadata {
        name: config.about_name.clone(),
        version: config.about_version.clone(),
        copyright: config.about_copyright.clone(),
        website: config.about_website.clone(),
        license: config.about_license.clone(),
        authors,
        comments: config.about_comments.clone(),
        credits: (!credits.is_empty()).then_some(credits),
        // Without an explicit icon, an unbundled app shows its folder icon
        icon: config.icon_file.as_deref().and_then(icon::load_menu_icon),
        ..Default::default()
    }
}

/// `_custom_edit_shortcuts`: see `BuiltMenu::custom_edit_shortcuts` (Windows only).
fn attach_menu_to_window(menu: &muda::Menu, _custom_edit_shortcuts: u8, _window: &tao::window::Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe { let _ = menu.init_for_hwnd(_window.hwnd() as _); }
        crate::accelerators::set_menu(_window.hwnd(), Some(menu), _custom_edit_shortcuts);
    }
    #[cfg(target_os = "macos")]
    {
        menu.init_for_nsapp();
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        // The GTK theme draws the menu bar transparent, so a background color set on the window
        // (e.g. a dark one) shows through while the labels keep the theme's text color: on a
        // light theme that is dark text over a dark background. Give the bar the theme's own
        // background so background and text stay readable and consistent with the title bar.
        // Applied regardless of the window background: with most themes it changes nothing,
        // but it makes the bar opaque on a transparent window.
        // Only on success: a failed init would leave no bar of this menu to style. The method
        // takes the menu by value, and cloning a muda::Menu only clones its handle.
        let menu_bar = menu
            .init_for_gtk_window(_window.gtk_window(), _window.default_vbox())
            .ok()
            .and_then(|_| menu.clone().gtk_menubar_for_gtk_window(_window.gtk_window()));
        if let Some(menu_bar) = menu_bar {
            let provider = gtk::CssProvider::new();
            let _ = provider.load_from_data(
                b"menubar { background-color: @theme_bg_color; color: @theme_fg_color; }",
            );
            menu_bar
                .style_context()
                .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    }
}

fn remove_menu_from_window(menu: &muda::Menu, _window: &tao::window::Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe { let _ = menu.remove_for_hwnd(_window.hwnd() as _); }
        crate::accelerators::set_menu(_window.hwnd(), None, 0);
    }
    #[cfg(target_os = "macos")]
    {
        menu.remove_for_nsapp();
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu.remove_for_gtk_window(_window.gtk_window());
    }
}

fn show_context_menu(
    menu: &muda::Menu,
    window: &tao::window::Window,
    pos: Option<(f64, f64)>,
) {
    use muda::ContextMenu;
    let position = pos.map(|(x, y)| muda::dpi::Position::Physical(muda::dpi::PhysicalPosition::new(x as i32, y as i32)));
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        let _ = unsafe { menu.show_context_menu_for_hwnd(window.hwnd() as _, position) };
    }
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowExtMacOS;
        let _ = unsafe { menu.show_context_menu_for_nsview(window.ns_view() as _, position) };
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        use gtk::prelude::Cast;
        let _ = menu.show_context_menu_for_gtk_window(window.gtk_window().upcast_ref::<gtk::Window>(), position);
    }
}

// --- Taskbar badge ---

fn set_badge_count(_window: &tao::window::Window, count: Option<u32>, _bg: [u8; 3], _fg: [u8; 3]) {
    #[cfg(target_os = "windows")]
    {
        set_badge_count_windows(_window, count, _bg, _fg);
    }
    #[cfg(target_os = "macos")]
    {
        set_badge_count_macos(count);
    }
    #[cfg(target_os = "linux")]
    {
        crate::window_ext::launcher::set_count(count);
    }
}

#[cfg(target_os = "windows")]
fn set_badge_count_windows(window: &tao::window::Window, count: Option<u32>, bg: [u8; 3], fg: [u8; 3]) {
    use tao::platform::windows::WindowExtWindows;
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::HWND;

    unsafe {
        let Ok(taskbar): Result<ITaskbarList3, _> =
            CoCreateInstance(&TaskbarList, None, CLSCTX_ALL)
        else {
            return;
        };

        let hwnd = HWND(window.hwnd() as *mut std::ffi::c_void);

        match count {
            None | Some(0) => {
                let _ = taskbar.SetOverlayIcon(hwnd, HICON::default(), None);
            }
            Some(n) => {
                if let Some(icon) = create_badge_icon(n, bg, fg) {
                    // Accessible description of the overlay, read by Narrator
                    let description = windows::core::HSTRING::from(n.to_string());
                    let _ = taskbar.SetOverlayIcon(hwnd, icon, &description);
                    let _ = DestroyIcon(icon);
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn create_badge_icon(count: u32, bg: [u8; 3], fg: [u8; 3]) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::core::w;

    let size: i32 = 32;

    unsafe {
        let hdc_screen = GetDC(None);
        let hdc = CreateCompatibleDC(Some(hdc_screen));
        ReleaseDC(None, hdc_screen);

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -(size);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;

        // --- Pass 1: Render white text on black to get coverage mask ---
        let mut text_bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let text_bmp = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut text_bits, None, 0).ok()?;
        let old_bmp = SelectObject(hdc, text_bmp.into());

        let text_pixels = text_bits as *mut u8;
        std::ptr::write_bytes(text_pixels, 0, (size * size * 4) as usize);

        let text = if count > 99 { "99+".to_string() } else { count.to_string() };
        let font_size = if text.len() <= 1 { 22 } else if text.len() == 2 { 18 } else { 14 };
        let font = CreateFontW(
            -font_size, 0, 0, 0,
            FW_BOLD.0 as i32,
            0, 0, 0,
            FONT_CHARSET(0),
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32 | FF_SWISS.0 as u32,
            w!("Segoe UI"),
        );
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));

        let wide_text: Vec<u16> = text.encode_utf16().collect();
        let nudge_x: i32 = 0;
        let nudge_y: i32 = -1;
        let mut rc = windows::Win32::Foundation::RECT {
            left: nudge_x, top: nudge_y, right: size + nudge_x, bottom: size + nudge_y,
        };
        DrawTextW(
            hdc,
            &mut wide_text.as_slice().to_vec(),
            &mut rc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP,
        );

        // Read text coverage from red channel (white text on black = coverage in any channel)
        let mut text_mask = vec![0u8; (size * size) as usize];
        for (i, item) in text_mask.iter_mut().enumerate().take((size * size) as usize) {
            *item = *text_pixels.add(i * 4 + 2); // R channel
        }

        SelectObject(hdc, old_font);
        SelectObject(hdc, old_bmp);
        let _ = DeleteObject(text_bmp.into());
        let _ = DeleteObject(font.into());

        // --- Pass 2: Compose final icon ---
        let mut final_bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let final_bmp = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut final_bits, None, 0).ok()?;

        let pixels = final_bits as *mut u8;
        std::ptr::write_bytes(pixels, 0, (size * size * 4) as usize);

        let cx = size as f32 / 2.0;
        let cy = size as f32 / 2.0;
        let r = cx - 0.5;
        // Sub-pixel shift for single digits (GDI centers on cell, not glyph)
        let text_shift_x: f32 = if text.len() == 1 { 1.0 } else { 0.0 };

        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= r {
                    let circle_alpha = (r - dist).min(1.0);

                    // Sample text mask with sub-pixel offset (bilinear interpolation)
                    let sx = x as f32 - text_shift_x;
                    let sy = y as f32;
                    let sx0 = sx.floor() as i32;
                    let sy0 = sy.floor() as i32;
                    let fx = sx - sx.floor();
                    let fy = sy - sy.floor();
                    let sample = |px: i32, py: i32| -> f32 {
                        if px >= 0 && px < size && py >= 0 && py < size {
                            text_mask[(py * size + px) as usize] as f32
                        } else {
                            0.0
                        }
                    };
                    let c00 = sample(sx0, sy0);
                    let c10 = sample(sx0 + 1, sy0);
                    let c01 = sample(sx0, sy0 + 1);
                    let c11 = sample(sx0 + 1, sy0 + 1);
                    let coverage = (c00 * (1.0 - fx) * (1.0 - fy)
                        + c10 * fx * (1.0 - fy)
                        + c01 * (1.0 - fx) * fy
                        + c11 * fx * fy) / 255.0;

                    // Blend: text foreground over background, then apply circle alpha
                    let out_r = fg[0] as f32 * coverage + bg[0] as f32 * (1.0 - coverage);
                    let out_g = fg[1] as f32 * coverage + bg[1] as f32 * (1.0 - coverage);
                    let out_b = fg[2] as f32 * coverage + bg[2] as f32 * (1.0 - coverage);
                    let alpha = (circle_alpha * 255.0) as u8;

                    let i = (y * size + x) as usize;
                    let idx = i * 4;
                    let p = pixels.add(idx);
                    // Premultiplied BGRA
                    *p = ((out_b * circle_alpha) as u8).min(alpha);
                    *p.add(1) = ((out_g * circle_alpha) as u8).min(alpha);
                    *p.add(2) = ((out_r * circle_alpha) as u8).min(alpha);
                    *p.add(3) = alpha;
                }
            }
        }

        let mask = CreateBitmap(size, size, 1, 1, None);
        let ii = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: final_bmp,
        };
        let icon = CreateIconIndirect(&ii).ok();

        let _ = DeleteObject(final_bmp.into());
        let _ = DeleteObject(mask.into());
        let _ = DeleteDC(hdc);

        icon
    }
}

#[cfg(target_os = "macos")]
fn set_badge_count_macos(count: Option<u32>) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let dock_tile = app.dockTile();
    match count {
        None | Some(0) => {
            dock_tile.setBadgeLabel(Some(&NSString::from_str("")));
        }
        Some(n) => {
            dock_tile.setBadgeLabel(Some(&NSString::from_str(&n.to_string())));
        }
    }
}

struct Tray {
    icon: tray_icon::TrayIcon,
    has_menu: bool,
    menu_on_left_click: bool,
}

impl Tray {
    /// Whether clicking with this button shows the tray menu (macOS, Windows).
    fn opens_menu(&self, button: tray_icon::MouseButton) -> bool {
        self.has_menu
            && match button {
                tray_icon::MouseButton::Left => self.menu_on_left_click,
                tray_icon::MouseButton::Right => true,
                tray_icon::MouseButton::Middle => false,
            }
    }
}

fn load_tray_icon(path: &str) -> Option<tray_icon::Icon> {
    // Windows: an .ico is loaded by the system at the small icon size, from its closest frame. Decoded as an image
    // its largest frame would be used, and the shell shrinks that one without filtering (jagged edges).
    #[cfg(target_os = "windows")]
    if path.to_ascii_lowercase().ends_with(".ico") {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) } as u32;
        if let Ok(icon) = tray_icon::Icon::from_path(path, Some((size, size))) {
            return Some(icon);
        }
    }
    let img = image::open(path).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()
}
pub(crate) fn handle_new_window_req(
    url: String,
    ctx: *mut std::ffi::c_void,
    cb: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char) -> i32,
) -> wry::NewWindowResponse {
    if let Ok(cstr) = CString::new(url) {
        unsafe { cb(ctx, cstr.as_ptr()) };
    }
    wry::NewWindowResponse::Deny
}

/// WebView2 serves custom schemes as `http://<scheme>.localhost/`: maps these URLs back to
/// `<scheme>://localhost/`, the form custom scheme handlers receive. Only the exact host
/// `<scheme>.localhost` (with an optional port) maps: `http://app.evil.com/` stays as it is.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn revert_custom_scheme_workaround(url: &str, schemes: &[String]) -> String {
    let Some(rest) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) else {
        return url.to_string();
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => (host, Some(port)),
        Some(_) => return url.to_string(),
        None => (authority, None),
    };
    let host = host.to_ascii_lowercase();
    let Some(scheme) = host
        .strip_suffix(".localhost")
        .and_then(|name| schemes.iter().find(|scheme| scheme.eq_ignore_ascii_case(name)))
    else {
        return url.to_string();
    };
    match port {
        Some(port) => format!("{scheme}://localhost:{port}{path}"),
        None => format!("{scheme}://localhost{path}"),
    }
}

/// Filled by the host (via `rustino_set_scheme_response`) while the custom scheme callback runs.
#[derive(Default)]
pub struct SchemeResponse {
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
}

pub(crate) fn handle_custom_scheme(
    url: String,
    ctx: *mut std::ffi::c_void,
    cb: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char, *mut SchemeResponse),
) -> wry::http::Response<Cow<'static, [u8]>> {
    let mut response = SchemeResponse::default();
    if let Ok(cstr) = CString::new(url) {
        unsafe { cb(ctx, cstr.as_ptr(), &mut response) };
    }
    let builder = wry::http::Response::builder();
    match response.body {
        Some(body) => builder
            .header(
                wry::http::header::CONTENT_TYPE,
                response.content_type.unwrap_or_else(|| "application/octet-stream".into()),
            )
            .body(Cow::Owned(body)),
        None => builder.status(404).body(Cow::Borrowed(&[][..])),
    }
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{handle_custom_scheme, handle_new_window_req, revert_custom_scheme_workaround, SchemeResponse};
    use std::ffi::CStr;
    use std::sync::atomic::{AtomicBool, Ordering};

    static CALLBACK_CALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn mock_cb(_ctx: *mut std::ffi::c_void, url: *const std::ffi::c_char) -> i32 {
        let c_str = unsafe { CStr::from_ptr(url) };
        assert_eq!(c_str.to_str().unwrap(), "https://example.com");
        CALLBACK_CALLED.store(true, Ordering::SeqCst);
        0
    }

    #[test]
    fn test_handle_new_window_req() {
        CALLBACK_CALLED.store(false, Ordering::SeqCst);
        let resp = handle_new_window_req(
            "https://example.com".to_string(),
            std::ptr::null_mut(),
            mock_cb,
        );
        assert!(CALLBACK_CALLED.load(Ordering::SeqCst));
        match resp {
            wry::NewWindowResponse::Deny => {}
            _ => panic!("Expected Deny"),
        }
    }

    unsafe extern "C" fn respond_cb(
        _ctx: *mut std::ffi::c_void,
        url: *const std::ffi::c_char,
        response: *mut SchemeResponse,
    ) {
        let url = unsafe { CStr::from_ptr(url) }.to_str().unwrap();
        assert_eq!(url, "app://localhost/index.html");
        let body = b"<html></html>";
        unsafe {
            crate::rustino_set_scheme_response(response, body.as_ptr(), body.len() as i32, c"text/html".as_ptr())
        };
    }

    unsafe extern "C" fn no_response_cb(
        _ctx: *mut std::ffi::c_void,
        _url: *const std::ffi::c_char,
        _response: *mut SchemeResponse,
    ) {
    }

    #[test]
    fn test_handle_custom_scheme_with_response() {
        let resp = handle_custom_scheme("app://localhost/index.html".to_string(), std::ptr::null_mut(), respond_cb);
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "text/html");
        assert_eq!(resp.body().as_ref(), b"<html></html>");
    }

    #[test]
    fn test_revert_custom_scheme_workaround() {
        let schemes = vec!["app".to_string()];
        let revert = |url: &str| revert_custom_scheme_workaround(url, &schemes);
        assert_eq!(revert("http://app.localhost/counter"), "app://localhost/counter");
        assert_eq!(revert("https://app.localhost/counter"), "app://localhost/counter");
        assert_eq!(revert("http://app.localhost"), "app://localhost");
        assert_eq!(revert("http://app.localhost:8080/a?b=c#d"), "app://localhost:8080/a?b=c#d");
        assert_eq!(revert("http://app.localhost?x=1"), "app://localhost?x=1");
        assert_eq!(revert("http://app.localhost#top"), "app://localhost#top");
        assert_eq!(revert("http://APP.localhost/"), "app://localhost/");
        assert_eq!(revert_custom_scheme_workaround("http://app.localhost/", &[]), "http://app.localhost/");
    }

    #[test]
    fn test_revert_custom_scheme_workaround_rejects_other_hosts() {
        let schemes = vec!["app".to_string()];
        let revert = |url: &str| revert_custom_scheme_workaround(url, &schemes);
        for url in [
            "https://example.com/",
            "http://app.evil.com/",
            "http://app.localhost.evil.com/",
            "http://app.localhostevil/",
            "http://evil.app.localhost/",
            "http://xapp.localhost/",
            "http://app.localhost:80x/",
            "http://app.localhost:/",
            "http://user@app.localhost/",
            "http://app.localhost@evil.com/",
            "ftp://app.localhost/",
            "app://localhost/",
            "https://example.com/?u=http://app.localhost/",
        ] {
            assert_eq!(revert(url), url);
        }
    }

    #[test]
    fn test_handle_custom_scheme_without_response_is_404() {
        let resp = handle_custom_scheme("app://localhost/missing".to_string(), std::ptr::null_mut(), no_response_cb);
        assert_eq!(resp.status(), 404);
        assert!(resp.body().is_empty());
    }

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

    #[test]
    fn webview2_browser_args_keep_wry_defaults() {
        let mut config = crate::config::WindowConfig::default();
        assert_eq!(super::webview2_browser_args(&config), None, "wry's own arguments");
        config.web_security_enabled = false;
        config.ignore_certificate_errors = true;
        assert_eq!(
            super::webview2_browser_args(&config).unwrap(),
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
             --autoplay-policy=no-user-gesture-required --disable-web-security --ignore-certificate-errors"
        );
        config.media_autoplay = false;
        config.web_security_enabled = true;
        assert_eq!(
            super::webview2_browser_args(&config).unwrap(),
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --ignore-certificate-errors"
        );
    }

    #[test]
    fn default_macos_menu_parses() {
        let defs: Vec<crate::menu::MenuItemDef> =
            serde_json::from_str(super::DEFAULT_MACOS_MENU).unwrap();
        assert_eq!(defs.len(), 2, "Edit and Window submenus");
    }

    /// The windows started while the app runs join its event loop (on macOS the only one there
    /// can be), and close with its first window. Opens hidden windows: needs WebView2.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_started_while_the_app_runs_join_its_event_loop() {
        use super::RustinoWindow;
        use crate::commands::RustinoCommand;
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;
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
