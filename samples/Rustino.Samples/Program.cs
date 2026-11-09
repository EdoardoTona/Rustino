using System.ComponentModel;
using System.Reactive.Linq;
using System.Text.Json;
using Microsoft.Extensions.Logging;
using Rustino.NET;
using Rustino.NET.Reactive;

// --- Logging ---
using var loggerFactory = LoggerFactory.Create(b => b.AddConsole().SetMinimumLevel(LogLevel.Warning));
var logger = loggerFactory.CreateLogger("Rustino");
var iconPath = Path.Combine(AppContext.BaseDirectory, "icon.png");
// macOS: monochrome template icon that follows the menu bar's colors
var trayIconPath = Path.Combine(AppContext.BaseDirectory, OperatingSystem.IsMacOS() ? "tray-template.png" : "icon.png");
var menuInitialized = false;
var (saveEnabled, saveRenamed, sidebarChecked, trayTitleCount) = (true, false, true, 0);

// --- Single instance: a second launch forwards its arguments here and exits ---
using var single = RustinoSingleInstance.Acquire("Rustino.Samples", new SingleInstanceOptions { Arguments = args, Logger = logger });
if (single.Status == SingleInstanceStatus.Forwarded)
{
    Console.WriteLine("Forwarded the arguments to the running instance.");
    return;
}
if (single.Status == SingleInstanceStatus.ForwardFailed)
{
    Console.Error.WriteLine("The running instance did not respond.");
    return;
}

// --- Notifications: Windows drops toasts for an appId that is not registered ---
RustinoWindow.RegisterNotificationAppId("Rustino", "Rustino", iconPath);

// --- Splashscreen ---
var splashPath = Path.Combine(AppContext.BaseDirectory, "splash.png");
if (RustinoSplashscreen.IsSupported) // Windows only
{
    using var splash = new RustinoSplashscreen(splashPath, 400, 400);
    Thread.Sleep(3000);
}

var window = new RustinoWindow();
// Windows opened from the page (see OpenWindow)
var windowCount = 0;

// --- Application menu bar ---
var appMenu = new RustinoMenu()
    .AddAppMenu(app => app // macOS only
        .AddPredefinedItem(PredefinedMenuItem.About)
        .AddSeparator()
        .AddItem("settings", "Settings…", accelerator: "CmdOrCtrl+,")
        .AddSeparator()
        .AddPredefinedItem(PredefinedMenuItem.Hide)
        .AddPredefinedItem(PredefinedMenuItem.Quit))
    .AddSubmenu("File", file => file
        .AddItem("new", "New", accelerator: "CmdOrCtrl+N")
        .AddItem("open", "Open...", accelerator: "CmdOrCtrl+O")
        .AddItem("save", "Save", accelerator: "CmdOrCtrl+S")
        .AddSeparator()
        .AddItem("exit", "Exit", accelerator: "Alt+F4"))
    .AddSubmenu("Edit", edit =>
    {
        // Undo/Redo are not available on Linux
        if (!OperatingSystem.IsLinux())
            edit.AddPredefinedItem(PredefinedMenuItem.Undo)
                .AddPredefinedItem(PredefinedMenuItem.Redo)
                .AddSeparator();
        edit.AddPredefinedItem(PredefinedMenuItem.Cut)
            .AddPredefinedItem(PredefinedMenuItem.Copy)
            .AddPredefinedItem(PredefinedMenuItem.Paste)
            .AddPredefinedItem(PredefinedMenuItem.SelectAll);
    })
    .AddSubmenu("View", view => view
        .AddCheckItem("sidebar", "Show Sidebar", isChecked: true, accelerator: "CmdOrCtrl+B")
        .AddCheckItem("statusbar", "Show Status Bar", isChecked: true)
        .AddSeparator()
        .AddSubmenu("Theme", theme => theme
            .AddItem("theme-light", "Light")
            .AddItem("theme-dark", "Dark")
            .AddItem("theme-system", "System Default")))
    .AddWindowMenu("Window", win => // macOS appends the open windows
    {
        // Minimize/Maximize are not available on Linux
        if (!OperatingSystem.IsLinux())
            win.AddPredefinedItem(PredefinedMenuItem.Minimize)
                .AddPredefinedItem(PredefinedMenuItem.Maximize)
                .AddSeparator();
        win.AddItem("center", "Center");
    })
    .AddHelpMenu("Help", help => help // macOS adds a search field
        .AddItem("docs", "Documentation", iconPath: iconPath)
        .AddSeparator()
        .AddItem("about", "About Rustino"));

var contextMenu = new RustinoMenu()
    .AddItem("ctx-cut", "Cut")
    .AddItem("ctx-copy", "Copy")
    .AddItem("ctx-paste", "Paste")
    .AddSeparator()
    .AddItem("ctx-select-all", "Select All");

var trayMenu = new RustinoMenu()
    .AddItem("tray-show", "Show Window")
    .AddItem("tray-hide", "Hide Window")
    .AddSeparator()
    .AddCheckItem("sidebar", "Show Sidebar", isChecked: true) // same id as View › Show Sidebar: kept in sync
    .AddSeparator()
    .AddItem("tray-quit", "Quit");

// --- Events (logged to console) ---
window.WindowClosing += (_, _) => Console.WriteLine("[Event] WindowClosing");
window.WindowClosed += (_, _) => Console.WriteLine("[Event] WindowClosed");
window.SizeChanged += (_, a) => { if (a is SizeEventArgs s) Console.WriteLine($"[Event] SizeChanged: {s.Width}x{s.Height}"); };
window.LocationChanged += (_, a) => { if (a is PointEventArgs p) Console.WriteLine($"[Event] LocationChanged: ({p.X}, {p.Y})"); };
window.FocusChanged += (_, f) => { if (f is bool b) Console.WriteLine($"[Event] FocusChanged: {b}"); };
window.PageLoaded += (_, a) =>
{
    if (a is PageLoadEventArgs pl)
    {
        Console.WriteLine($"[Event] PageLoad: {(pl.IsStarted ? "Started" : "Finished")} - {pl.Url}");
        if (pl.IsFinished && !menuInitialized)
        {
            menuInitialized = true;
            window.SetMenu(appMenu);
            window.SetTrayIcon(trayIconPath, tooltip: "Rustino Feature Showcase", menu: trayMenu,
                isTemplateIcon: OperatingSystem.IsMacOS(), menuOnLeftClick: false);
        }
    }
};

// --- Reactive observables (logged to console) ---
window.WhenSizeChangedThrottled(TimeSpan.FromMilliseconds(300))
    .Subscribe(s => Console.WriteLine($"[Reactive] Resized: {s.Width}x{s.Height}"));

window.WhenPageLoadCompleted()
    .Subscribe(e => Console.WriteLine($"[Reactive] PageLoad completed: {e.Url}"));

window.WhenFocusChangedDistinct()
    .Subscribe(f => Console.WriteLine($"[Reactive] Focus: {(f ? "Gained" : "Lost")}"));

window.WhenWebMessageWithPrefix("cmd:")
    .Subscribe(cmd => Console.WriteLine($"[Reactive] Command: {cmd}"));

window.WhenWindowClosed
    .Subscribe(_ => Console.WriteLine("[Reactive] Window closed"));

// --- Native window features ---
window.ThemeChanged += (_, theme) =>
{
    Console.WriteLine($"[Event] ThemeChanged: {theme}");
    window.ExecuteScript($"showTheme('{theme}')");
};
window.ScaleFactorChanged += (_, a) =>
    Console.WriteLine($"[Event] ScaleFactorChanged: {a.ScaleFactor} ({a.Width}x{a.Height})");
// Deep links (see AttachDeepLinks below) and, on macOS, files and URLs opened through the app bundle
window.UrlsOpened += (_, urls) =>
{
    Console.WriteLine($"[Event] UrlsOpened: {string.Join(", ", urls)}");
    Log($"Opened: {string.Join(", ", urls)}");
};
window.Navigating += (_, e) => Console.WriteLine($"[Event] Navigating: {(e.Url.Length > 80 ? e.Url[..80] + "…" : e.Url)}");
window.UnhandledCallbackException += (_, e) => Console.Error.WriteLine($"[Error] {e.CallbackName}: {e.Exception.Message}");

// --- Later launches of the sample (dotnet run -- some args) ---
single.SecondInstanceStarted += (_, e) =>
{
    Console.WriteLine($"[SingleInstance] PID {e.ProcessId} in {e.WorkingDirectory}: {string.Join(" ", e.Args)}");
    Log($"Second instance (PID {e.ProcessId}) args: {(e.Args.Length == 0 ? "(none)" : string.Join(" ", e.Args))}");
};
// macOS: a click on the Dock icon brings back the window hidden in the tray
window.Reopened += (_, hasVisibleWindows) =>
{
    Console.WriteLine($"[Event] Reopened (visible windows: {hasVisibleWindows})");
    if (!hasVisibleWindows) window.SetVisible(true).Focus();
};

// The tab bar takes the place of the title bar, next to the traffic lights (macOS) or the window
// buttons drawn by Rustino (Windows, Linux), as high as the tabs
window.SetMacTitleBarStyle(MacTitleBarStyle.Overlay).SetTitleBarOverlay(true).SetTitleBarOverlayHeight(40);

// --- Webview: file drop, downloads, document title ---
var saveToDownloads = false;
window.FileDrop += (_, e) =>
{
    if (e.Type == FileDropEventType.Drop)
        Log($"Dropped at ({e.X}, {e.Y}):\n{string.Join("\n", e.Paths)}");
};
// The window title follows the page's <title>
window.DocumentTitleChanged += (_, title) => window.SetTitle(title);
window.DownloadStarting += (_, e) =>
{
    // Without a destination the user chooses it in the save dialog
    if (saveToDownloads) e.DestinationPath = e.SuggestedPath;
    Log($"Download starting: {Path.GetFileName(e.SuggestedPath)}{(saveToDownloads ? " → " + e.SuggestedPath : "")}");
};
window.DownloadCompleted += (_, e) => Log(e.Success ? $"Download saved: {e.Path}" : "Download not saved");

// --- Menu clicks ---
window.MenuItemClicked += (_, id) =>
{
    Console.WriteLine($"[Menu] {id}");
    switch (id)
    {
        case "exit" or "tray-quit": window.Close(); break;
        case "tray-show": window.SetVisible(true).Focus(); break;
        case "tray-hide": window.SetVisible(false); break;
        case "center": window.Center(); break;
        default: Log($"Menu: {id}"); break;
    }
};

window.MenuItemCheckedChanged += (_, e) =>
{
    if (e.Id == "sidebar") sidebarChecked = e.IsChecked;
    Log($"Menu: {e.Id} {(e.IsChecked ? "checked" : "unchecked")}");
};

// Left click shows the window, right click opens the tray menu (menuOnLeftClick: false)
window.TrayIconClicked += (_, e) =>
{
    Console.WriteLine($"[Tray] {e.Button} click at ({e.X}, {e.Y})");
    if (e.Button == TrayMouseButton.Left)
        window.SetVisible(true).Focus();
};

// --- IPC message handler ---
window.WebMessageReceived += (_, msg) =>
{
    Console.WriteLine($"[IPC] {msg}");
    HandleMessage(msg);
};

void HandleMessage(string msg)
{
    switch (msg)
    {
        // Window state
        case "minimize": window.Minimize(); break;
        case "maximize": window.Maximize(); break;
        case "restore": window.Restore(); break;
        case "fullscreen": window.SetFullscreen(true); break;
        case "exit-fullscreen": window.SetFullscreen(false); break;
        case "chromeless": SetTitleBar("Chromeless"); break;
        case "decorated": SetTitleBar("Overlay"); break;
        case "topmost-on": window.SetTopMost(true); break;
        case "topmost-off": window.SetTopMost(false); break;
        case "zoom-in": window.SetZoom(1.5); break;
        case "zoom-out": window.SetZoom(0.75); break;
        case "zoom-reset": window.SetZoom(1.0); break;
        case "size-small": window.SetSize(640, 480); break;
        case "size-large": window.SetSize(1200, 800); break;
        case "center": window.Center(); break;
        case "move-tl": window.SetPosition(50, 50); break;
        case "close": window.Close(); break;

        // Query state
        case "get-state":
            var (w, h) = window.GetSize();
            var (x, y) = window.GetPosition();
            Log($"Size: {w}x{h}, Pos: ({x},{y}), Min: {window.IsMinimized}, Max: {window.IsMaximized}, FS: {window.IsFullscreen}");
            break;

        // JS interop
        case "ping":
            window.SendWebMessage("pong from C#!");
            break;
        case "get-time":
            Log($"Server time: {DateTime.Now:HH:mm:ss}");
            break;

        // Launch: activation and external URLs
        case "activate-later":
            window.SetVisible(false);
            Log("Hidden: Activate() in 3 s");
            Task.Delay(3000).ContinueWith(_ => window.Activate());
            break;
        case var ext when ext.StartsWith("open-external:"):
            var url = ext["open-external:".Length..];
            Log($"OpenExternal({url}): {RustinoWindow.OpenExternal(url)}");
            break;

        // Native window features
        case var wv when wv.StartsWith("webview:"):
            HandleWebViewMessage(wv["webview:".Length..]);
            break;

        case var native when native.StartsWith("native:"):
            HandleNativeMessage(native["native:".Length..]);
            break;

        // Badges
        case "badge-clear":
            window.ClearBadge();
            Log("Badge cleared");
            break;

        // Dialogs
        case "open-single":
            var file = window.ShowOpenFileDialog(
                title: "Select a File",
                filters: [
                    new FileFilter("Images", "jpg", "jpeg", "png", "gif", "bmp"),
                    new FileFilter("Documents", "pdf", "doc", "docx", "txt"),
                    new FileFilter("All Files", "*")
                ]);
            Log(file != null ? $"Opened: {string.Join(", ", file)}" : "Dialog cancelled");
            break;
        case "open-multi":
            var files = window.ShowOpenFileDialog(title: "Select Multiple Files", multiSelect: true,
                filters: [new FileFilter("All Files", "*")]);
            Log(files != null ? $"Opened {files.Length} file(s):\n{string.Join("\n", files)}" : "Dialog cancelled");
            break;
        case "save":
            var savePath = window.ShowSaveFileDialog(title: "Save File As", defaultPath: "document.txt",
                filters: [new FileFilter("Text Files", "txt"), new FileFilter("CSV Files", "csv"), new FileFilter("All Files", "*")]);
            Log(savePath != null ? $"Save to: {savePath}" : "Dialog cancelled");
            break;
        case "folder-single":
            var folder = window.ShowSelectFolderDialog(title: "Select a Folder");
            Log(folder != null ? $"Folder: {string.Join(", ", folder)}" : "Dialog cancelled");
            break;
        case "folder-multi":
            var folders = window.ShowSelectFolderDialog(title: "Select Multiple Folders", multiSelect: true);
            Log(folders != null ? $"Folders:\n{string.Join("\n", folders)}" : "Dialog cancelled");
            break;

        // Notifications
        case "notify-basic":
            RustinoWindow.ShowNotification("Hello from Rustino!", "This is a basic notification.", appId: "Rustino");
            Log("Notification sent");
            break;
        case "notify-detailed":
            RustinoWindow.ShowNotification("Download Complete", "report-2026.pdf saved to Downloads.", appId: "Rustino");
            Log("Notification sent");
            break;
        case "notify-alert":
            RustinoWindow.ShowNotification("Build Failed", "3 errors found in Program.cs.", appId: "Rustino");
            Log("Notification sent");
            break;

        // Context menu
        case "show-context-menu":
            window.ShowContextMenu(contextMenu);
            break;

        // Menu items, changed without rebuilding the menu
        case "menu-toggle-save":
            saveEnabled = !saveEnabled;
            window.SetMenuItemEnabled("save", saveEnabled);
            Log($"File › Save {(saveEnabled ? "enabled" : "disabled")}");
            break;
        case "menu-rename-save":
            saveRenamed = !saveRenamed;
            window.SetMenuItemText("save", saveRenamed ? "Save All" : "Save");
            break;
        case "menu-toggle-sidebar":
            sidebarChecked = !sidebarChecked;
            window.SetMenuItemChecked("sidebar", sidebarChecked);
            break;

        // Tray title (macOS, Linux)
        case "tray-title":
            window.SetTrayTitle($"{++trayTitleCount}");
            break;
        case "tray-title-clear":
            window.SetTrayTitle(null);
            break;

        // Monitors
        case "get-monitors":
            var monitors = window.GetMonitors();
            var current = window.GetCurrentMonitor();
            var jsonOpts = new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
            var data = JsonSerializer.Serialize(new { monitors, currentName = current?.Name }, jsonOpts);
            window.ExecuteScript($"updateMonitors({data})");
            break;

        // Reactive demo
        case "cmd:reload":
        case "cmd:save":
            Log($"Reactive command received: {msg}");
            break;

        default:
            if (msg.StartsWith("badge:"))
            {
                var parts = msg[6..].Split(':');
                if (int.TryParse(parts[0], out var count))
                {
                    string? bg = parts.Length > 1 ? parts[1] : null;
                    string? fg = parts.Length > 2 ? parts[2] : null;
                    window.SetBadgeCount(count, background: bg, foreground: fg);
                    Log($"Badge set to {count} (bg={bg ?? "default"}, fg={fg ?? "default"})");
                }
            }
            else if (msg.StartsWith("move-to:") && int.TryParse(msg[8..], out var idx))
            {
                var mons = window.GetMonitors();
                if (idx >= 0 && idx < mons.Length)
                {
                    var target = mons[idx];
                    var (sw, sh) = window.GetSize();
                    window.SetPosition(target.X + (target.Width - sw) / 2, target.Y + (target.Height - sh) / 2);
                }
            }
            else
            {
                Log($"Echo: {msg}");
            }
            break;
    }
}

void HandleNativeMessage(string msg)
{
    var parts = msg.Split(':');
    var on = parts.Length > 1 && parts[1] == "1";
    switch (parts[0])
    {
        case "theme":
            window.SetTheme(Enum.Parse<WindowTheme>(parts[1]));
            break;
        case "progress":
            var state = Enum.Parse<ProgressBarState>(parts[1]);
            int? value = parts.Length > 2 ? int.Parse(parts[2]) : null;
            window.SetProgressBar(state, value);
            break;
        case "attention":
            // No effect while the app is focused: switch to another app within 3 seconds
            var type = Enum.Parse<UserAttentionType>(parts[1]);
            Log($"Requesting attention ({type}) in 3 s: switch to another app");
            Task.Delay(3000).ContinueWith(_ => window.RequestUserAttention(type));
            break;
        case "beep": window.Beep(); break;
        case "click-through":
            Log("The window ignores the mouse for 5 s");
            window.SetIgnoreCursorEvents(true);
            Task.Delay(5000).ContinueWith(_ => window.SetIgnoreCursorEvents(false));
            break;
        case "titlebar": SetTitleBar(parts[1]); break;
        case "shadow": window.SetShadow(on); break;
        case "skip-taskbar": window.SetSkipTaskbar(on); break;
        case "content-protection": window.SetContentProtection(on); break;
        case "all-workspaces": window.SetVisibleOnAllWorkspaces(on); break;
        case "closable": window.SetClosable(on); break;
        case "minimizable": window.SetMinimizable(on); break;
        case "maximizable": window.SetMaximizable(on); break;
        case "always-on-bottom": window.SetAlwaysOnBottom(on); break;
        case "info":
            Log($"Theme: {window.Theme}, scale factor: {window.ScaleFactor}");
            break;
        case "open-window": OpenWindow(); break;
    }
}

// The window joins the event loop of the main window: its thread only waits for the closing
void OpenWindow()
{
    var number = ++windowCount;
    var other = new RustinoWindow()
        .SetTitle($"Window {number}")
        .SetUseOsDefaultSize(false)
        .SetSize(480, 320)
        .Load("data:text/html," + Uri.EscapeDataString($"""
            <body style="font-family:system-ui;padding:16px 24px">
              <h2>Window {number}</h2>
              <p>It runs in the event loop of the main window and closes with it.</p>
              <button onclick="window.ipc.postMessage('hello')">Say hello to the main window</button>
            </body>
            """));
    other.WebMessageReceived += (_, message) => Log($"Window {number}: {message}");
    other.WindowClosed += (_, _) => Log($"Window {number} closed");
    new Thread(() =>
    {
        other.WaitForClose();
        other.Dispose();
    }) { IsBackground = true }.Start();
    Log($"Window {number} opened");
}

void HandleWebViewMessage(string msg)
{
    switch (msg)
    {
        case "message-info":
            Log($"ShowMessage: {window.ShowMessage("Rustino", "An information message.")}");
            break;
        case "message-question":
            Log($"ShowMessage: {window.ShowMessage("Unsaved changes", "Save the document before closing?", RustinoDialogButtons.YesNoCancel, RustinoDialogIcon.Question)}");
            break;
        case "message-warning":
            Log($"ShowMessage: {window.ShowMessage("Connection lost", "The server doesn't answer.", RustinoDialogButtons.RetryCancel, RustinoDialogIcon.Warning)}");
            break;
        case "message-error":
            Log($"ShowMessage: {window.ShowMessage("Copy failed", "The file is in use.", RustinoDialogButtons.AbortRetryIgnore, RustinoDialogIcon.Error)}");
            break;
        case "message-background":
            // From another thread too the message box is modal for the window
            Task.Run(() => window.ShowMessage("Background", "Shown from a background thread.")).ContinueWith(t => Log($"ShowMessage: {t.Result}"));
            break;
        case "save-to-downloads:1": saveToDownloads = true; break;
        case "save-to-downloads:0": saveToDownloads = false; break;
        case "print": window.Print(); break;
        case "reload": window.Reload(); break;
        case "devtools": window.OpenDevTools(); break;
        case "clear-data": window.ClearBrowsingData(); Log("Browsing data cleared"); break;
        case "cookie-set":
            window.SetCookie(new RustinoCookie("visits", DateTime.Now.ToString("HH:mm:ss")) { Domain = "example.com", Path = "/", Expires = DateTimeOffset.Now.AddDays(7) });
            Log("Cookie set for example.com");
            break;
        case "cookie-list":
            // WebMessageReceived is a webview event: on Windows cookies are read once it returns
            Task.Run(() => window.GetCookies()).ContinueWith(t =>
                Log(t.Result.Length == 0 ? "No cookies" : string.Join("\n", t.Result.Select(c => $"{c.Name}={c.Value} ({c.Domain}{c.Path}, expires {c.Expires?.ToString("g") ?? "at the end of the session"})"))));
            break;
        case "cookie-delete":
            window.DeleteCookie(new RustinoCookie("visits", "") { Domain = "example.com", Path = "/" });
            Log("Cookie deleted");
            break;
    }
}

// Chromeless, or decorated with a MacTitleBarStyle (macOS) or the overlay title bar (Windows, Linux)
void SetTitleBar(string mode)
{
    var chromeless = mode == "Chromeless";
    window.SetChromeless(chromeless);
    if (!chromeless)
        window.SetMacTitleBarStyle(Enum.Parse<MacTitleBarStyle>(mode)).SetTitleBarOverlay(mode == "Overlay");
}

void Log(string text)
{
    var escaped = JsonSerializer.Serialize(text);
    window.ExecuteScript($"appendLog({escaped})");
}

// --- Build & run ---

window
    .SetLogger(logger)
    .SetTitle("Rustino — Feature Showcase")
    .SetIconFile(iconPath)
    .SetAboutName("Rustino Showcase")
    .SetAboutVersion("1.0")
    .SetAboutCopyright("MIT License")
    .SetAboutComments("Native desktop windows with embedded web views, powered by Rust.")
    .SetAboutWebsite("https://github.com/EdoardoTona/Rustino")
    .SetUseOsDefaultSize(false)
    .SetSize(1100, 800)
    .SetMinSize(600, 400)
    // .SetMaxSize(1920, 1080)
    .SetResizable(true)
    .Center()
    .SetTopMost(false)
    .SetChromeless(false)
    .SetTransparent(false)
    .SetBackgroundColor(26, 26, 46)
    .SetDevToolsEnabled(true)
    .SetJavascriptClipboardAccessEnabled(false)
    .SetIgnoreCertificateErrorsEnabled(true)
    .SetWebSecurityEnabled(false)
    .SetMediaAutoplayEnabled(true)
    .SetZoomHotkeysEnabled(true)
    // Right click shows the app's context menu (see the contextmenu listener in the page)
    .SetContextMenuEnabled(false)
    .SetScrollBarStyle(ScrollBarStyle.FluentOverlay)
    .SetFileDropEnabled(true)
    .AddInitScript("""
        window.addEventListener('message', (e) => {
            appendLog('From C#: ' + e.data);
        });
    """)
    .Load("data:text/html," + Uri.EscapeDataString(Html()));

// rustino-sample:// URLs from this launch and from later ones raise UrlsOpened.
// Try: dotnet run -- rustino-sample://open?id=42
window.AttachDeepLinks(single, "rustino-sample");
// Each forwarded launch restores and activates the window
single.MainWindow = window;

window.WaitForClose();

// The page of the showcase: index.html, copied next to the executable
static string Html() => File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "index.html"));
