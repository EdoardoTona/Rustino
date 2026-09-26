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

All `.Set*()`, `.Center()`, `.Load()`, and `.WaitForClose()` calls remain the same.

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
| `SetDevToolsEnabled(bool)` | Enable browser developer tools |
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

### Runtime (post-run)

| Method | Description |
|---|---|
| `Minimize()` | Minimize the window |
| `Maximize()` | Maximize the window |
| `Restore()` | Restore from minimized/maximized |
| `SetFullscreen(bool)` | Enter/exit fullscreen |
| `SetVisible(bool)` | Show/hide the window |
| `Focus()` | Bring focus to the window |
| `Close()` | Close the window |
| `ExecuteScript(string)` | Evaluate JavaScript in the webview |
| `SendWebMessage(string)` | Post a message to the webview |
| `SetZoom(double)` | Set webview zoom factor |
| `SetBadgeCount(int?, string?, string?)` | Set taskbar/dock badge with optional bg/fg hex colors |
| `ClearBadge()` | Remove the taskbar/dock badge |
| `WaitForClose()` | Block until the window is closed |
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

Set a numeric badge on the taskbar icon (Windows) or dock icon (macOS):

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
