# Rustino

Cross-platform native desktop windows with embedded web views, powered by **Rust**. 

Inspired by [Photino.NET](https://tryphotino.io).

Rustino replaces Photino's C++ native layer with Rust, using [wry](https://github.com/nicbarker/gaia) for the webview and [tao](https://github.com/nicbarker/gaia) for window management — the same libraries that power [Tauri](https://tauri.app).

## Architecture

```
Your .NET App
    └── RustinoWindow (C#)          ← Rustino.NET
            └── P/Invoke
                └── rustino_native   ← Rustino.Native (Rust cdylib)
                    ├── wry           → WebView2 (Windows)
                    ├── wry           → WKWebView (macOS)
                    └── wry           → WebKitGTK (Linux)
```

## Quick Start

```csharp
using Rustino.NET;

var window = new RustinoWindow();
window
    .SetTitle("My App")
    .SetUseOsDefaultSize(false)
    .SetSize(1280, 800)
    .SetResizable(true)
    .Center()
    .Load(new Uri("https://example.com"));

window.WaitForClose();
```

## Migrating from Photino

The API is identical. Change two things:

```diff
- using Photino.NET;
+ using Rustino.NET;

- var window = new PhotinoWindow();
+ var window = new RustinoWindow();
```

All `.Set*()`, `.Center()`, `.Load()`, and `.WaitForClose()` calls remain the same. The types of `ShowMessage` follow the same rename: `PhotinoDialogButtons`, `PhotinoDialogIcon` and `PhotinoDialogResult` become `RustinoDialogButtons`, `RustinoDialogIcon` and `RustinoDialogResult`, with the same values.

For Blazor apps, replace `Photino.Blazor` with [`Rustino.Blazor`](#blazor-hybrid-rustinoblazor):

```diff
- using Photino.Blazor;
+ using Rustino.Blazor;

- var builder = PhotinoBlazorAppBuilder.CreateDefault(args);
+ var builder = RustinoBlazorAppBuilder.CreateDefault(args);
```

## API Reference

### Configuration (pre-run)

| Method | Description |
|---|---|
| `SetTitle(string)` | Set the window title |
| `SetSize(int, int)` | Set window dimensions in pixels |
| `SetMinSize(int, int)` | Set minimum window size |
| `SetMaxSize(int, int)` | Set maximum window size |
| `SetPosition(int, int)` | Set window position |
| `SetUseOsDefaultSize(bool)` | Use OS default window size |
| `SetResizable(bool)` | Allow/prevent window resizing |
| `SetTopMost(bool)` | Keep window above all others |
| `SetChromeless(bool)` | Remove window decorations (title bar, borders) |
| `SetTransparent(bool)` | Enable transparent background |
| `SetMaximized(bool)` | Start maximized |
| `SetBackgroundColor(r, g, b, a)` | Set webview background color |
| `SetIconFile(string)` | Set window icon from .ico/.png file path |
| `SetIcon(Stream)` | Set window icon from a .NET stream (e.g. embedded resource) |
| `Center()` | Center window on the primary monitor |
| `SetDevToolsEnabled(bool)` | Enable the web inspector (right click › Inspect, F12 on Windows, `OpenDevTools()`; on macOS see [Developer tools](#webview)) |
| `SetContextMenuEnabled(bool)` | The browser's context menu on right click (on by default) |
| `SetBrowserControlsEnabled(bool)` | Windows: the browser's shortcuts, such as F5, Ctrl+R, Ctrl+F and Ctrl+P (on by default) |
| `SetScrollBarStyle(ScrollBarStyle)` | Windows: `Default` or `FluentOverlay`, the overlay scroll bars of Windows 11 |
| `SetAcceptFirstMouse(bool)` | macOS: the click that activates the window also reaches the page |
| `SetBackForwardGesturesEnabled(bool)` | Swipes go back and forward in the history (off by default) |
| `SetFileDropEnabled(bool)` | Files dropped on the window raise `FileDrop` with their paths (see [Webview](#webview)) |
| `SetJavascriptClipboardAccessEnabled(bool)` | Allow JS clipboard access |
| `SetIgnoreCertificateErrorsEnabled(bool)` | Ignore SSL certificate errors |
| `SetWebSecurityEnabled(bool)` | Enable/disable web security (CORS, etc.) |
| `SetMediaAutoplayEnabled(bool)` | Allow media to autoplay |
| `SetZoomHotkeysEnabled(bool)` | Enable Ctrl+/- zoom hotkeys |
| `SetUserAgent(string)` | Set custom user agent string |
| `SetUserDataFolder(string)` | Set webview data folder path |
| `AddInitScript(string)` | Add JavaScript to run before page loads |
| `RegisterCustomSchemeHandler(string, handler)` | Serve `scheme://` requests from .NET (see [Custom Schemes](#custom-schemes)) |
| `Load(Uri)` / `Load(string)` | Navigate to a URL or local file |
| `LogVerbosity` | Set log verbosity (0 = silent) |

`Load()` creates the native instance. Creation settings other than `LogVerbosity` can still be changed after
`Load()` and before the first `WaitForClose()`; Rustino applies them to the native configuration. Set `LogVerbosity`
before `Load()` because it is passed to the native constructor. Settings that affect window or webview creation
throw `InvalidOperationException` if changed after `WaitForClose()` starts. Runtime commands such as `SetMenu()`,
`ExecuteScript()` and `SendWebMessage()` can be sent before `WaitForClose()` and are delivered in order once the
window and webview exist.

### Runtime (post-run)

| Method | Description |
|---|---|
| `Minimize()` | Minimize the window |
| `Maximize()` | Maximize the window |
| `Restore()` | Restore from minimized/maximized |
| `SetFullscreen(bool)` | Enter/exit fullscreen |
| `SetVisible(bool)` | Show/hide the window |
| `Focus()` | Bring focus to the window |
| `Activate(string? activationToken = null)` | Show and restore the window, then request focus. On Linux, pass the launcher activation token when available. |
| `Close()` | Close the window |
| `ExecuteScript(string)` | Evaluate JavaScript in the webview |
| `SendWebMessage(string)` | Post a message to the webview |
| `SetZoom(double)` | Set webview zoom factor |
| `Print()` | Open the system print dialog for the page |
| `Reload()` | Reload the page (a page loaded from an HTML string is loaded again) |
| `OpenDevTools()` / `CloseDevTools()` | Open or close the web inspector (needs `SetDevToolsEnabled(true)`; closing isn't supported on Windows; on macOS see [Developer tools](#webview)) |
| `ClearBrowsingData()` | Delete the cookies, the cache and the storage of every site |
| `GetCookies(string?)` / `SetCookie(RustinoCookie)` / `DeleteCookie(RustinoCookie)` | Read and change the webview's cookies (see [Webview](#webview)) |
| `SetBadgeCount(int?, string?, string?)` | Set taskbar/dock badge with optional bg/fg hex colors |
| `ClearBadge()` | Remove the taskbar/dock badge |
| `WaitForClose()` | Block until the window is closed; throws `RustinoException` if native window or webview creation fails |
| `Dispose()` | Release native resources (`RustinoWindow` implements `IDisposable`) |

### Custom Schemes

Serve content for a custom URL scheme directly from .NET (same signature as Photino). Register handlers before `WaitForClose()`; returning `null` produces a 404:

```csharp
window.RegisterCustomSchemeHandler("app", (sender, scheme, url, out contentType) =>
{
    contentType = "text/html";
    return new MemoryStream("<h1>Hello from .NET</h1>"u8.ToArray());
});
window.Load("app://localhost/");
```

On Windows, WebView2 sees custom schemes as `http://<scheme>.localhost/`; handlers still receive the original `<scheme>://...` URLs.

### Dialogs

Native cross-platform file dialogs (powered by [rfd](https://github.com/PolyMeilex/rfd)):

```csharp
// Open file (single)
string[]? files = window.ShowOpenFileDialog(
    title: "Select an image",
    filters: [new FileFilter("Images", "jpg", "png", "gif")]);

// Open files (multi-select)
string[]? files = window.ShowOpenFileDialog(
    title: "Select files",
    multiSelect: true);

// Save file
string? path = window.ShowSaveFileDialog(
    title: "Save as",
    defaultPath: "document.pdf",
    filters: [new FileFilter("PDF", "pdf")]);

// Select folder
string[]? folders = window.ShowSelectFolderDialog(
    title: "Choose output directory");
```

All dialogs return `null` when canceled. File filters use the format `new FileFilter("Name", "ext1", "ext2", ...)`.

Message boxes use the same signature as Photino's `ShowMessage`:

```csharp
var answer = window.ShowMessage("Unsaved changes", "Save the document before closing?",
    RustinoDialogButtons.YesNoCancel, RustinoDialogIcon.Question);
if (answer == RustinoDialogResult.Yes) Save();
```

Buttons: `Ok`, `OkCancel`, `YesNo`, `YesNoCancel`, `RetryCancel`, `AbortRetryIgnore`. Icons: `Info`, `Warning`, `Error`, `Question` (shown as `Info` on macOS). Closing the box without a button returns `Cancel`, or `Ok` when `Ok` is the only button.

While the window runs, every dialog is modal for it: a sheet on macOS, owned by the window on Windows (which can't be clicked until the dialog closes), transient for it on Linux. They can be called from any thread, including the event handlers; before the window runs they have no parent. Message boxes use `MessageBoxW` on Windows (localized buttons), `NSAlert` on macOS and `GtkMessageDialog` on Linux; before the window runs, macOS shows a system alert instead of an `NSAlert`. On Linux file dialogs go through the XDG desktop portal.

### Notifications

Native cross-platform toast notifications (powered by [notify-rust](https://github.com/hoodie/notify-rust)):

```csharp
// Static — no window instance required
RustinoWindow.ShowNotification("Download Complete", "Your file has been saved.");

// With icon (file path)
RustinoWindow.ShowNotification("Alert", "Something happened", iconPath: "/path/to/icon.png");

// With icon (embedded resource stream)
using var stream = Assembly.GetExecutingAssembly()
    .GetManifestResourceStream("MyApp.notify.png")!;
RustinoWindow.ShowNotification("Alert", "Something happened", stream);
```

Uses WinRT Toast (Windows), NSUserNotification (macOS), and D-Bus (Linux).

#### Windows: register your `appId`

On Windows, a toast is shown under the AppUserModelID passed as `appId`. Windows only displays toasts for an id it knows (a packaged app, a Start Menu shortcut carrying the id, or a registry registration) and **silently drops** the others, while `ShowNotification` still returns `true`. Without `appId`, notifications fall back to the Windows PowerShell id and show its name and icon.

Unpackaged apps (a plain `.exe`, a dotnet tool) should register their id once at startup:

```csharp
RustinoWindow.RegisterNotificationAppId("MyCompany.MyApp", "My App", iconPath: @"C:\path\to\icon.png");
RustinoWindow.ShowNotification("Hello", "Shown as My App", appId: "MyCompany.MyApp");
```

This writes `HKCU\Software\Classes\AppUserModelId\<appId>` (display name and icon; no admin rights, no shortcut). The icon must be an image file (`.png`/`.ico`) that stays at that path; registering again without `iconPath` removes the previous icon. `appId` must be a valid [AppUserModelID](https://learn.microsoft.com/windows/win32/shell/appids) such as `CompanyName.ProductName` (at most 128 characters, no spaces or backslashes); otherwise the call returns `false` on every platform. On macOS and Linux a valid id is accepted without writing anything.

### Menus

Native cross-platform application menus and context menus (powered by [muda](https://github.com/nicbarker/gaia)):

```csharp
// Application menu bar
var menu = new RustinoMenu()
    .AddSubmenu("File", file => file
        .AddItem("new", "New", accelerator: "CmdOrCtrl+N", iconPath: "new.png")
        .AddItem("open", "Open...", accelerator: "CmdOrCtrl+O")
        .AddSeparator()
        .AddItem("exit", "Exit"))
    .AddSubmenu("Edit", edit => edit
        .AddPredefinedItem(PredefinedMenuItem.Undo)
        .AddPredefinedItem(PredefinedMenuItem.Redo)
        .AddSeparator()
        .AddPredefinedItem(PredefinedMenuItem.Copy)
        .AddPredefinedItem(PredefinedMenuItem.Paste, label: "Paste here")
        .AddSeparator()
        .AddCheckItem("wordwrap", "Word Wrap", isChecked: true, accelerator: "Alt+Z"))
    .AddWindowMenu("Window", win => win
        .AddPredefinedItem(PredefinedMenuItem.Minimize)
        .AddPredefinedItem(PredefinedMenuItem.Maximize))
    .AddHelpMenu("Help", help => help
        .AddItem("about", "About"));

window.SetMenu(menu);

// Context menu (right-click)
var ctx = new RustinoMenu()
    .AddItem("cut", "Cut")
    .AddItem("copy", "Copy")
    .AddItem("paste", "Paste");

window.ShowContextMenu(ctx);

// Handle clicks
window.MenuItemClicked += (_, id) => Console.WriteLine($"Clicked: {id}");

// A click toggles a check item: its new state comes after MenuItemClicked
window.MenuItemCheckedChanged += (_, e) => Console.WriteLine($"{e.Id}: {e.IsChecked}");

// Change items without rebuilding the menu
window.SetMenuItemEnabled("open", false);
window.SetMenuItemChecked("wordwrap", false);
window.SetMenuItemText("open", "Open Recent...");

// Remove menu bar
window.RemoveMenu();
```

The `SetMenuItem*` methods change every item with that id in the menu bar and in the tray menu, until the menu is set again. Items with the same id stay in sync when the user toggles one of them. A context menu is built from its `RustinoMenu` at every `ShowContextMenu`, so it always shows the values of the definition: build it when you show it, with the current values (for a check item, the state received from `MenuItemCheckedChanged`).

`iconPath` shows an image next to the label, scaled to the menu's icon size; if it can't be read, the item has no icon.

On macOS, the system lists the open windows in the menu added with `AddWindowMenu` and adds a search field to the one added with `AddHelpMenu`; on Windows and Linux both are normal submenus. Until `SetMenu`, macOS shows a standard menu bar: the application menu, Edit and Window.

Predefined items run a native OS action and don't raise `MenuItemClicked`. On macOS, a custom menu bar replaces the default one: include the predefined Edit items, otherwise Cmd+C/V/X/A stop working in the webview.

On macOS, `SetMenu` also prepends the standard application menu (About, Hide, Hide Others, Show All, Quit), shown with the app name, so your first submenu (e.g. File) stays visible. Replace it with `AddAppMenu` (ignored on Windows and Linux):

```csharp
var menu = new RustinoMenu()
    .AddAppMenu(app => app
        .AddPredefinedItem(PredefinedMenuItem.About)
        .AddSeparator()
        .AddItem("settings", "Settings…", accelerator: "CmdOrCtrl+,")
        .AddSeparator()
        .AddPredefinedItem(PredefinedMenuItem.Quit))
    .AddSubmenu("File", file => file.AddItem("open", "Open..."));
```

The predefined `About` item opens the native About panel with the window icon and the values of `SetAboutName`, `SetAboutVersion`, `SetAboutCopyright`, `SetAboutComments`, `AddAboutAuthor`, `SetAboutWebsite` and `SetAboutLicense` (the macOS panel shows the last four as credits). Without `SetAboutName`, macOS uses the app name shown in the menu bar. The values are read when a menu is set: after changing them (or the icon) at runtime, call `SetMenu` again. For a fully custom About, use a normal item and handle `MenuItemClicked`.

Support depends on the platform (muda):

| Predefined item | macOS | Windows | Linux |
|---|---|---|---|
| `About` | OK | OK, no icon | OK |
| `Cut`, `Copy`, `Paste`, `SelectAll` | OK | OK | X11 only¹ |
| `Undo`, `Redo`, `Minimize`, `Maximize`, `Hide`, `CloseWindow`, `Quit` | OK | OK | omitted |
| `Fullscreen`, `HideOthers`, `ShowAll`, `Services`, `BringAllToFront` | OK | shown, does nothing | omitted |

"Omitted" items are not added to the menu. ¹ Clicking the item sends the shortcut through libxdo; on Wayland it does nothing (the keyboard shortcut still works in the webview).

On Windows, Ctrl+C/X/V/A/Z/Y go to the webview, which handles them natively, unless a custom item uses them: the item then replaces the native shortcut. If the menu also has the predefined item with the same shortcut (Copy, Cut, Paste, Select All, Undo, Redo), the shortcut stays with the webview.

### System Tray

Native cross-platform system tray icon with optional context menu (powered by [tray-icon](https://github.com/nicbarker/gaia)):

```csharp
// Tray icon with tooltip and context menu
var trayMenu = new RustinoMenu()
    .AddItem("show", "Show Window")
    .AddItem("hide", "Hide Window")
    .AddSeparator()
    .AddItem("quit", "Quit");

window.SetTrayIcon("icon.png", tooltip: "My App", menu: trayMenu);

// From embedded resource (Stream)
using var stream = Assembly.GetExecutingAssembly()
    .GetManifestResourceStream("MyApp.tray.png")!;
window.SetTrayIcon(stream, tooltip: "My App", menu: trayMenu);

// macOS: a monochrome template icon that follows the light or dark menu bar, text next to
// the icon, and the menu only on right click
window.SetTrayIcon("tray-template.png", tooltip: "My App", menu: trayMenu,
    title: "3", isTemplateIcon: true, menuOnLeftClick: false);
window.SetTrayTitle("4");

// Handle tray icon clicks
window.TrayIconClicked += (_, e) =>
{
    if (e.Button == TrayMouseButton.Left)
        window.SetVisible(true);
};

// Remove tray icon
window.RemoveTrayIcon();
```

`TrayIconClicked` is raised once per click, with the button and the cursor position in physical pixels. It's raised when the button is released, or when it's pressed if the click opens the tray menu (on macOS the menu takes the release). By default both left and right click open the menu: with `menuOnLeftClick: false` left clicks only raise `TrayIconClicked`.

A template icon uses only the image's alpha channel, so a colored icon turns into a solid silhouette: use it with a black shape on a transparent background.

| | macOS | Windows | Linux |
|---|---|---|---|
| `TrayIconClicked` | OK | OK | never raised¹ |
| `tooltip` | OK | OK | not supported |
| `title`, `SetTrayTitle` | OK | not supported | only on some desktops |
| `isTemplateIcon` | OK | ignored | ignored |
| `menuOnLeftClick` | OK | OK | ignored: any click opens the menu |

¹ On Linux the tray icon is an AppIndicator (libayatana-appindicator), which doesn't report clicks: put every action in the tray menu.

### Taskbar Badge

Set a numeric badge on the taskbar icon (Windows) or dock icon (macOS, Linux):

```csharp
// Set badge with default colors (red background, white text)
window.SetBadgeCount(5);

// Set badge with custom colors (Windows only, hex format)
window.SetBadgeCount(5, background: "#4A154B", foreground: "#FFFFFF");

// Clear the badge
window.ClearBadge();
```

On Windows, this renders an overlay icon on the taskbar button using a 32px anti-aliased circle with bold Segoe UI text. Numbers above 99 display as "99+". The `background` and `foreground` parameters accept `#RRGGBB` hex strings and default to `#E01E5A` (red) and `#FFFFFF` (white).

On macOS, the native `dockTile.setBadgeLabel` API is used — color parameters are ignored as the OS controls badge appearance.

On Linux, the count goes to the dock through the Unity LauncherEntry D-Bus API (Ubuntu Dock, Dash to Dock, KDE Plasma). The dock finds the app by its `.desktop` file: `<executable name>.desktop` unless you call `SetDesktopFileName("com.example.App.desktop")`.

### Monitors

Enumerate connected displays with position, resolution, and DPI scale factor:

```csharp
// Get all monitors
MonitorInfo[] monitors = window.GetMonitors();
foreach (var m in monitors)
    Console.WriteLine($"{m.Name}: {m.Width}x{m.Height} at ({m.X},{m.Y}), scale={m.ScaleFactor}, primary={m.IsPrimary}");

// Get the monitor containing this window
MonitorInfo? current = window.GetCurrentMonitor();

// DPI-aware positioning: center window on a specific monitor
var target = monitors.First(m => !m.IsPrimary);
var (w, h) = window.GetSize();
window.SetPosition(
    target.X + (target.Width - w) / 2,
    target.Y + (target.Height - h) / 2);
```

`MonitorInfo` properties: `Name`, `X`, `Y`, `Width`, `Height`, `ScaleFactor`, `IsPrimary`.

### Native Window Features

```csharp
window
    .SetTheme(WindowTheme.Dark)                     // title bar, native controls, prefers-color-scheme
    .SetMacTitleBarStyle(MacTitleBarStyle.Overlay)  // macOS: the page extends under the traffic lights
    .SetMacTrafficLightPosition(14, 12);

window.ThemeChanged += (_, theme) => Console.WriteLine($"Now {theme}");

// Later, while the window runs
window.SetProgressBar(ProgressBarState.Normal, 40);   // taskbar button / Dock icon
window.RequestUserAttention(UserAttentionType.Critical);
```

The platform limits below hold both before the window runs and while it runs.

| Method / property | Description |
|---|---|
| `SetTheme(WindowTheme)` | `System`, `Light` or `Dark`. Windows applies it to the window (dark title bar) and to WebView2; macOS and Linux to the whole app |
| `Theme` | Current theme, `Light` or `Dark` |
| `ScaleFactor` | Physical pixels per logical pixel on the window's monitor |
| `SetProgressBar(ProgressBarState, int?)` / `ClearProgressBar()` | Progress on the taskbar button (Windows), the Dock icon (macOS) or the dock icon (Linux, LauncherEntry: see [Taskbar Badge](#taskbar-badge)) |
| `RequestUserAttention(UserAttentionType)` / `CancelUserAttentionRequest()` | Flashes the taskbar button, bounces the Dock icon or sets the urgency hint, until the app is focused |
| `SetShadow(bool)` | Window shadow (Windows: chromeless windows; macOS: all windows) |
| `SetSkipTaskbar(bool)` | No taskbar button, for apps that live in the tray. On macOS the app leaves the Dock and the app switcher |
| `SetContentProtection(bool)` | Keeps the window out of screenshots and recordings (Windows, macOS) |
| `SetVisibleOnAllWorkspaces(bool)` | Shows the window on every virtual desktop (macOS, Linux) |
| `SetClosable(bool)` / `SetMinimizable(bool)` / `SetMaximizable(bool)` | Enable the title bar buttons. Linux: only `SetClosable`, as a request the window manager may ignore; minimize and maximize can't be disabled |
| `SetAlwaysOnBottom(bool)` | Keeps the window below the others, replacing `SetTopMost`. Linux: a request to the window manager, not supported on Wayland |
| `SetIgnoreCursorEvents(bool)` | Mouse clicks go through the window, for overlays |
| `SetMacTitleBarStyle(MacTitleBarStyle)` | macOS: `Default`, `Transparent` or `Overlay` (no title, the page under the traffic lights, like Slack or VS Code). `SetChromeless(false)` brings the style back |
| `SetMacTrafficLightPosition(double, double)` | macOS: position of the traffic lights, in logical pixels |
| `SetDesktopFileName(string)` | Linux: `.desktop` file of the app, for the dock badge and progress |
| `SetDragRegionsEnabled(bool)` | Pre-run: drag regions and edge resizing from the page (on by default, see below) |
| `DragWindow()` / `DragResizeWindow(ResizeDirection)` | Move or resize the window with the mouse (call them while the left button is down) |

| Event | Args | Description |
|---|---|---|
| `ThemeChanged` | `WindowTheme` | The theme changed, by the system or by `SetTheme` |
| `ScaleFactorChanged` | `ScaleFactorChangedEventArgs` | The window moved to a monitor with another scale (`.ScaleFactor`, `.Width`, `.Height`) |
| `UrlsOpened` | `string[]` | macOS: the app was asked to open files or URLs (file associations and URL schemes of the app bundle) |
| `Reopened` | `bool` | macOS: Dock icon clicked; `false` when no window is visible, e.g. hidden in the tray |

Each event also has an observable: `WhenThemeChanged`, `WhenScaleFactorChanged`, `WhenUrlsOpened`, `WhenReopened`.

On Linux with the X11 backend WebKitGTK restyles the page when the theme changes, but doesn't fire the `change` event of `matchMedia('(prefers-color-scheme: dark)')`: scripts that need to know should listen to `ThemeChanged` instead.

#### Drag regions for chromeless windows

The elements with `data-rustino-drag-region` move the window, and a double click maximizes it. The attribute applies to the element itself, not to its children, so the buttons of a custom title bar stay clickable:

```html
<header data-rustino-drag-region>
  <span data-rustino-drag-region>My App</span>
  <button onclick="window.ipc.postMessage('close')">✕</button>
</header>
```

On Windows and Linux the page covers the resize borders of chromeless windows: its outer 6 pixels resize the window instead. macOS keeps its own resize borders. With `MacTitleBarStyle.Overlay` the page's title bar needs the attribute too.

The drag region script talks to the native side with `window.ipc.postMessage` messages that start with `__rustino:` (like the print script on Windows and macOS, see [Webview](#webview)): they never reach `WebMessageReceived`, so don't use that prefix for your own messages. Any page loaded in the window can send them, and so move, resize or maximize it: if the window shows untrusted pages, call `SetDragRegionsEnabled(false)` before `WaitForClose()` (no script, and the `__rustino:` messages reach `WebMessageReceived` like the others, except `__rustino:print`, which only prints).

#### Tray apps on macOS

A click on the Dock icon raises `Reopened`: show the window again when it was hidden in the tray.

```csharp
window.Reopened += (_, hasVisibleWindows) =>
{
    if (!hasVisibleWindows) window.SetVisible(true).Focus();
};
```

### Webview

Options and events that make the page behave like an app rather than a browser:

```csharp
window
    .SetContextMenuEnabled(false)                   // no browser menu on right click
    .SetBrowserControlsEnabled(false)               // Windows: no F5, Ctrl+R, Ctrl+F, Ctrl+P
    .SetScrollBarStyle(ScrollBarStyle.FluentOverlay) // Windows 11 scroll bars
    .SetFileDropEnabled(true);

// Files dropped on the window, with their full paths (HTML5 drag and drop only gives the names)
window.FileDrop += (_, e) =>
{
    if (e.Type == FileDropEventType.Drop)
        OpenFiles(e.Paths);
};

// The page's <title> in the title bar
window.DocumentTitleChanged += (_, title) => window.SetTitle(title);

// Downloads: by default the user chooses where to save each file in the native save dialog
window.DownloadStarting += (_, e) =>
{
    if (e.Url.EndsWith(".exe")) e.Cancel = true;                 // refuse it
    else if (saveToDownloads) e.DestinationPath = e.SuggestedPath; // save without asking
};
window.DownloadCompleted += (_, e) => Console.WriteLine(e.Success ? $"Saved {e.Path}" : $"Not saved: {e.Url}");
```

Without `SetContextMenuEnabled(false)` right click opens the browser's menu. On macOS and Linux turning it off adds an init script that prevents the default of `contextmenu` events: the page still gets them, e.g. to call `ShowContextMenu`. `SetBrowserControlsEnabled(false)` leaves the menu accelerators and the page's own shortcuts working; WKWebView and WebKitGTK have no browser shortcuts.

**File drop.** With `SetFileDropEnabled(true)`, `FileDrop` reports `Enter` (with the paths), `Over`, `Drop` (with the paths) and `Leave`; positions are in logical (CSS) pixels from the top-left corner of the page. The page doesn't get the dropped files, and the webview doesn't navigate to them. Drags that carry no files (e.g. elements dragged within the page) stay with the page on macOS and Linux; on Windows the webview stops receiving drops altogether, so the page's HTML5 drag and drop doesn't work while file drop is on (a WebView2 limitation of wry).

**Downloads.** `DownloadStarting` runs before the file is written: `SuggestedPath` is where a browser would save it (the Downloads folder, with a name that doesn't replace an existing file). Leave `DestinationPath` null to let the user choose in the save dialog (modal for the window), set it to save there without asking (an existing file is replaced), or set `Cancel`. Don't show dialogs from this handler. The file is downloaded to a temporary folder and moved to its destination when complete, so the save dialog can stay open while it downloads. `DownloadCompleted` reports every download that `DownloadStarting` let through: `Path` is null when the download failed or the user canceled the dialog. Without these events WKWebView wouldn't download at all; WebView2 no longer shows its download bar, nor asks before a page downloads more than one file. WKWebView and WebKitGTK start only the last of several downloads that the same script starts (e.g. consecutive `click()` calls on `<a download>` links): let a moment pass between them, e.g. with `setTimeout`.

**Cookies.** `GetCookies()` returns all the cookies, `GetCookies(url)` those sent to that URL; `SetCookie` adds or replaces a cookie (same name, domain and path), `DeleteCookie` removes it:

```csharp
window.SetCookie(new RustinoCookie("session", token) { Domain = "example.com", Path = "/", Secure = true, HttpOnly = true });
var session = window.GetCookies("https://example.com/").FirstOrDefault(c => c.Name == "session");
```

On Windows, WebView2 applies `SetCookie` and `DeleteCookie` a moment later: a `GetCookies` right after them can still return the previous cookies. On Windows, `GetCookies` also can't run within a webview event (`WebMessageReceived`, `Navigating`, `PageLoaded`, ...), where it throws `InvalidOperationException`: WebView2 answers only after the event returns. Call it afterwards, e.g. `var cookies = await Task.Run(() => window.GetCookies());` in an async handler.

**Printing.** `Print()` and the page's `window.print()` open the system print dialog on every platform. On Windows and macOS an init script sends `window.print()` to it: WebView2 would show its print preview inside the window, cut by small windows (the system dialog of Windows has no preview), and WKWebView would do nothing.

**Developer tools.** With `SetDevToolsEnabled(true)`, right click › Inspect (and F12 on Windows) open the web inspector, and so does `OpenDevTools()`: in its own window on Windows, docked in the window on Linux. On macOS the inspector inside the app needs a private WebKit API, which can get an app rejected from the Mac App Store, so the native library includes it only when built with the `devtools` feature (see [Building from Source](#building-from-source)). Without it, `SetDevToolsEnabled(true)` makes the page inspectable from Safari's Develop menu (macOS 13.3 or later), while `OpenDevTools()` and `CloseDevTools()` do nothing.

### State Queries

| Property | Description |
|---|---|
| `IsMinimized` | Whether the window is minimized |
| `IsMaximized` | Whether the window is maximized |
| `IsFullscreen` | Whether the window is in fullscreen |
| `GetPosition()` | Returns `(X, Y)` position |
| `GetSize()` | Returns `(Width, Height)` size |
| `GetMonitors()` | Returns all connected `MonitorInfo[]` |
| `GetCurrentMonitor()` | Returns `MonitorInfo?` for the monitor containing the window |

### Events

| Event | Args | Description |
|---|---|---|
| `WindowClosing` | `CancelEventArgs` | Fired before close (set `Cancel = true` to prevent) |
| `WindowClosed` | `EventArgs` | Fired after the window is destroyed |
| `UnhandledCallbackException` | `NativeCallbackExceptionEventArgs` | A handler, observable observer, or logger threw while Rustino.Native was calling into .NET (`.CallbackName`, `.Exception`) |
| `SizeChanged` | `SizeEventArgs` | Fired on resize (`.Width`, `.Height`) |
| `LocationChanged` | `PointEventArgs` | Fired on move (`.X`, `.Y`) |
| `FocusChanged` | `bool` | Fired on focus/blur |
| `WebMessageReceived` | `string` | Fired when JS calls `window.ipc.postMessage(msg)` |
| `WebMessageReceivedWithSource` | `WebMessageEventArgs` | Same messages with the sending page's URL (`.Message`, `.SourceUrl`): check it if the webview can navigate to other sites |
| `PageLoaded` | `PageLoadEventArgs` | Fired on page load start/finish (`.IsStarted`, `.Url`) |
| `Navigating` | `NavigationEventArgs` | Fired before navigation (`.Url`, set `Cancel = true` to block) |
| `MenuItemClicked` | `string` | Fired when a menu item is clicked (the item's ID) |
| `MenuItemCheckedChanged` | `MenuItemCheckedEventArgs` | Fired after `MenuItemClicked` when the user toggles a check item (`.Id`, `.IsChecked`) |
| `TrayIconClicked` | `TrayIconClickedEventArgs` | Fired once per click on the tray icon (`.Button`, `.X`, `.Y`); never on Linux |
| `FileDrop` | `FileDropEventArgs` | Files dragged over the window and dropped, with their full paths (`.Type`, `.Paths`, `.X`, `.Y`); needs `SetFileDropEnabled(true)` |
| `DocumentTitleChanged` | `string` | The page's `<title>` changed |
| `DownloadStarting` | `DownloadStartingEventArgs` | A download starts (`.Url`, `.SuggestedPath`; set `.DestinationPath` or `.Cancel`) |
| `DownloadCompleted` | `DownloadCompletedEventArgs` | A download ended (`.Url`, `.Path`, `.Success`) |

Exceptions thrown from a native callback are caught before they can cross the Rust/.NET boundary. Rustino logs
them through the configured `ILogger` and raises `UnhandledCallbackException`; exceptions thrown by that event
are also caught. A closing-handler exception allows the window to close, a navigation-handler exception allows
the navigation, and a download-starting exception cancels the download. A custom-scheme handler exception leaves
the response empty, which Rustino returns as HTTP 404.

### Observable Streams (IObservable&lt;T&gt;)

All events are also available as `IObservable<T>` properties for reactive programming (no System.Reactive dependency required):

| Property | Type | Description |
|---|---|---|
| `WhenSizeChanged` | `IObservable<(int Width, int Height)>` | Size change stream |
| `WhenLocationChanged` | `IObservable<(int X, int Y)>` | Position change stream |
| `WhenFocusChanged` | `IObservable<bool>` | Focus/blur stream |
| `WhenWebMessageReceived` | `IObservable<string>` | JS message stream |
| `WhenWebMessageReceivedWithSource` | `IObservable<WebMessageEventArgs>` | JS message stream with the sending page's URL |
| `WhenPageLoaded` | `IObservable<PageLoadEventArgs>` | Page load stream |
| `WhenNavigating` | `IObservable<NavigationEventArgs>` | Navigation stream |
| `WhenWindowClosed` | `IObservable<EventArgs>` | Window closed stream |
| `WhenMenuItemClicked` | `IObservable<string>` | Menu item click stream |
| `WhenMenuItemCheckedChanged` | `IObservable<MenuItemCheckedEventArgs>` | Check item toggle stream |
| `WhenTrayIconClicked` | `IObservable<TrayIconClickedEventArgs>` | Tray icon click stream |
| `WhenFileDrop` | `IObservable<FileDropEventArgs>` | File drag and drop stream |
| `WhenDocumentTitleChanged` | `IObservable<string>` | Document title stream |
| `WhenDownloadStarting` | `IObservable<DownloadStartingEventArgs>` | Download start stream |
| `WhenDownloadCompleted` | `IObservable<DownloadCompletedEventArgs>` | Download end stream |

All streams complete automatically when the window closes or is disposed.

### Rustino.NET.Reactive (companion package)

For System.Reactive operators, add the `Rustino.NET.Reactive` package:

```csharp
using System.Reactive.Linq;
using Rustino.NET.Reactive;

// Throttled resize
window.WhenSizeChangedThrottled(TimeSpan.FromMilliseconds(200))
    .Subscribe(size => Console.WriteLine($"{size.Width}x{size.Height}"));

// Message routing by prefix
window.WhenWebMessageWithPrefix("cmd:")
    .Subscribe(cmd => HandleCommand(cmd));

// Page load completion only
window.WhenPageLoadCompleted()
    .Subscribe(e => Console.WriteLine($"Loaded: {e.Url}"));

// Throttled move events
window.WhenLocationChangedThrottled(TimeSpan.FromMilliseconds(100))
    .Subscribe(pos => Console.WriteLine($"Moved to {pos.X},{pos.Y}"));

// Distinct focus changes
window.WhenFocusChangedDistinct()
    .Subscribe(focused => Console.WriteLine($"Focus: {focused}"));
```

## Blazor Hybrid (Rustino.Blazor)

The `Rustino.Blazor` package hosts Razor components in a Rustino window, like Photino.Blazor. Components run in .NET and render into the native webview; the app is served from `app://localhost/`.

```csharp
using Rustino.Blazor;

var builder = RustinoBlazorAppBuilder.CreateDefault(args);
builder.Services.AddSingleton<MyService>();   // regular dependency injection
builder.RootComponents.Add<App>("#app");

var app = builder.Build();
app.MainWindow.SetTitle("My Blazor App");     // the underlying RustinoWindow
app.Run();
```

Use the `Microsoft.NET.Sdk.Razor` SDK and put a host page in `wwwroot/index.html`:

```html
<!DOCTYPE html>
<html>
<head>
    <base href="/" />
</head>
<body>
    <div id="app">Loading…</div>
    <script src="_framework/blazor.webview.js"></script>
</body>
</html>
```

Inside components:

- `@inject RustinoWindow Window` gives access to the whole native API (dialogs, notifications, menus, window state, …).
- Native events (`SizeChanged`, `MenuItemClicked`, …) are raised outside Blazor's dispatcher: update state with `InvokeAsync(StateHasChanged)`.
- `IJSRuntime` and `[JSInvokable]` work as in any Blazor app.
- `@inject HttpClient Http` reads app files (e.g. `Http.GetStringAsync("data.json")`; missing files are `404 Not Found`) and forwards other requests to the network.
- Only the app's own pages (`app://localhost/`) can talk to the components: messages from other sites the webview navigates to, or from frames of other origins embedded in the app, are ignored.

Static files come from `wwwroot` next to the executable (published apps) or from the project during development. Pass an `IFileProvider` to `CreateDefault` to serve them from somewhere else (e.g. embedded resources).

See [`src/Rustino.Samples.Blazor`](src/Rustino.Samples.Blazor) for a complete example.

## Building from Source

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (1.80+)
- [.NET SDK](https://dotnet.microsoft.com/download) (10.0+)
- **Windows**: WebView2 runtime (pre-installed on Windows 10/11)
- **macOS**: Xcode Command Line Tools
- **Linux**: `libgtk-3-dev libwebkit2gtk-4.1-dev`

### Build

```bash
# Build the Rust native library
cd src/Rustino.Native
cargo build --release
# ...or, on macOS, with the web inspector inside the app (private WebKit API, see Developer tools)
cargo build --release --features devtools

# Build the .NET wrapper
cd ../Rustino.NET
dotnet build

# Run a sample
cd ../Rustino.Samples
dotnet run

# Run the Blazor sample
cd ../Rustino.Samples.Blazor
dotnet run
```

## Cross-Platform Support

| Platform | WebView Engine | Native Library |
|---|---|---|
| Windows x64/ARM64 | WebView2 (Chromium) | `rustino_native.dll` |
| macOS x64/ARM64 | WKWebView (WebKit) | `librustino_native.dylib` |
| Linux x64/ARM64 | WebKitGTK | `librustino_native.so` |

## License

MIT — see [LICENSE](LICENSE).

Inspired by and API-compatible with [Photino](https://tryphotino.io), originally created by TryPhotino (Apache-2.0).
