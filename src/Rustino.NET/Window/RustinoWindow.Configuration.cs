using System.Reflection;

namespace Rustino.NET;

// Settings of the window and its webview. Most can change while the window runs; the others
// (creation settings) throw once it started.
public partial class RustinoWindow
{
    // --- Original builder methods ---

    public RustinoWindow SetUseOsDefaultSize(bool useDefault)
    {
        SetCreationOnly(ref _useOsDefaultSize, useDefault, nameof(SetUseOsDefaultSize),
            static (instance, value) => RustinoDllImports.rustino_set_use_os_default_size(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetSize(int width, int height)
    {
        ThrowIfDisposed();
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(width);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(height);
        _width = width;
        _height = height;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_size(handle, width, height);
        return this;
    }

    public RustinoWindow SetTitle(string title)
    {
        ThrowIfDisposed();
        _title = title;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_title(handle, title);
        return this;
    }

    public RustinoWindow SetResizable(bool resizable)
    {
        ThrowIfDisposed();
        _resizable = resizable;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_resizable(handle, resizable ? 1 : 0);
        return this;
    }

    public RustinoWindow SetTopMost(bool topMost)
    {
        ThrowIfDisposed();
        _topmost = topMost;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_topmost(handle, topMost ? 1 : 0);
        return this;
    }

    public RustinoWindow SetJavascriptClipboardAccessEnabled(bool enabled)
    {
        SetCreationOnly(ref _clipboardEnabled, enabled, nameof(SetJavascriptClipboardAccessEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_clipboard_enabled(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetDevToolsEnabled(bool enabled)
    {
        SetCreationOnly(ref _devToolsEnabled, enabled, nameof(SetDevToolsEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_devtools_enabled(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetIgnoreCertificateErrorsEnabled(bool enabled)
    {
        SetCreationOnly(ref _ignoreCertErrors, enabled, nameof(SetIgnoreCertificateErrorsEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_ignore_cert_errors(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetWebSecurityEnabled(bool enabled)
    {
        SetCreationOnly(ref _webSecurityEnabled, enabled, nameof(SetWebSecurityEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_web_security_enabled(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetIconFile(string path) => SetIconFile(path, tempFile: false);

    // Written to a temporary file, deleted when the icon is replaced and by Dispose
    public RustinoWindow SetIcon(Stream icon)
    {
        ThrowIfDisposed();
        return SetIconFile(WriteTempFile(icon, "rustino_icon"), tempFile: true);
    }

    private RustinoWindow SetIconFile(string path, bool tempFile)
    {
        try
        {
            ThrowIfDisposed();
            _iconFile = path;
            if (Native is { } handle)
            {
                RustinoDllImports.rustino_set_icon_file(handle, path);
                if (OperatingSystem.IsMacOS())
                {
                    SetMacDockIcon(path);
                }
            }
        }
        catch
        {
            if (tempFile) DeleteFile(path);
            throw;
        }
        // The macOS Dock icon is loaded again from _iconFile at every page load
        DeleteFile(Interlocked.Exchange(ref _iconTempFile, tempFile ? path : null));
        return this;
    }

    public RustinoWindow Center()
    {
        ThrowIfDisposed();
        _center = true;
        if (Native is { } handle)
            RustinoDllImports.rustino_center(handle);
        return this;
    }

    /// <summary>
    /// Navigates to <paramref name="uri"/>. <c>file:</c> URIs and relative URIs are local files, as with
    /// <see cref="Load(string)"/>.
    /// </summary>
    public RustinoWindow Load(Uri uri)
    {
        ArgumentNullException.ThrowIfNull(uri);
        ThrowIfDisposed();
        var url = !uri.IsAbsoluteUri ? LocalFileUrl(ResolveFile(uri.OriginalString))
            : uri.IsFile && !uri.IsUnc ? LocalFileUrl(Path.GetFullPath(uri.LocalPath), uri.Query + uri.Fragment)
            : uri.AbsoluteUri;
        RustinoDllImports.rustino_navigate_to_url(EnsureNative(), url);
        return this;
    }

    /// <summary>
    /// Navigates to an absolute URL (<c>https:</c>, a custom scheme, ...), to the HTML of a
    /// <c>data:text/html,</c> URL, or to a local file (a path or a <c>file:</c> URL). A relative path is
    /// resolved against the current directory, or against the app's directory when the file isn't in the
    /// current directory. Local files are served from <c>rustino-file://localhost/&lt;full path&gt;</c>, with
    /// the other files of their folder and its subfolders: <c>file:</c> pages can't send messages to the host.
    /// </summary>
    public RustinoWindow Load(string pathOrUrl)
    {
        ArgumentException.ThrowIfNullOrEmpty(pathOrUrl);
        ThrowIfDisposed();
        if (pathOrUrl.StartsWith("data:text/html,", StringComparison.OrdinalIgnoreCase))
        {
            var html = Uri.UnescapeDataString(pathOrUrl["data:text/html,".Length..]);
            RustinoDllImports.rustino_navigate_to_string(EnsureNative(), html);
        }
        else
        {
            RustinoDllImports.rustino_navigate_to_url(EnsureNative(), ToUrl(pathOrUrl));
        }
        return this;
    }

    // Absolute URLs stay as they are; paths and file: URLs are local files. Absolute paths
    // (C:\app\index.html, /app/index.html) parse as file URIs too.
    internal string ToUrl(string pathOrUrl)
    {
        if (!Uri.TryCreate(pathOrUrl, UriKind.Absolute, out var uri) || (uri.IsFile && Path.IsPathRooted(pathOrUrl)))
            return LocalFileUrl(ResolveFile(pathOrUrl));
        return uri.IsFile && !uri.IsUnc ? LocalFileUrl(Path.GetFullPath(uri.LocalPath), uri.Query + uri.Fragment) : pathOrUrl;
    }

    private static string ResolveFile(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (!File.Exists(fullPath) && !Path.IsPathRooted(path))
        {
            var appPath = Path.GetFullPath(path, AppContext.BaseDirectory);
            if (File.Exists(appPath))
                fullPath = appPath;
        }
        return fullPath;
    }

    // --- New builder methods (Phase 3-5) ---

    public RustinoWindow SetTransparent(bool transparent)
    {
        SetCreationOnly(ref _transparent, transparent, nameof(SetTransparent),
            static (instance, value) => RustinoDllImports.rustino_set_transparent(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetChromeless(bool chromeless)
    {
        ThrowIfDisposed();
        _decorations = !chromeless;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_decorations(handle, _decorations ? 1 : 0);
        return this;
    }

    public RustinoWindow SetPosition(int x, int y)
    {
        ThrowIfDisposed();
        _position = (x, y);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_position(handle, x, y);
        return this;
    }

    public RustinoWindow SetMinSize(int width, int height)
    {
        ThrowIfDisposed();
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(width);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(height);
        _minSize = (width, height);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_min_size(handle, width, height);
        return this;
    }

    public RustinoWindow SetMaxSize(int width, int height)
    {
        ThrowIfDisposed();
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(width);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(height);
        _maxSize = (width, height);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_max_size(handle, width, height);
        return this;
    }

    public RustinoWindow SetBackgroundColor(byte r, byte g, byte b, byte a = 255)
    {
        ThrowIfDisposed();
        _backgroundColor = (r, g, b, a);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_background_color(handle, r, g, b, a);
        return this;
    }

    public RustinoWindow SetUserAgent(string userAgent)
    {
        SetCreationOnly(ref _userAgent, userAgent, nameof(SetUserAgent), RustinoDllImports.rustino_set_user_agent);
        return this;
    }

    public RustinoWindow SetUserDataFolder(string path)
    {
        SetCreationOnly(ref _userDataFolder, path, nameof(SetUserDataFolder), RustinoDllImports.rustino_set_user_data_folder);
        return this;
    }

    // Without SetUserDataFolder, WebView2 keeps its data next to the executable, in <exe>.WebView2. With
    // `dotnet app.dll` the executable is dotnet.exe, whose folder is in Program Files and shared by every app,
    // and an app installed in Program Files can't write there either: WebView2 then fails with "access denied".
    // In both cases the data goes to %LOCALAPPDATA%\<app>\WebView2 instead; apps that could already write next
    // to their executable keep their data there.
    private static string? DefaultUserDataFolder()
    {
        if (!OperatingSystem.IsWindows())
            return null;
        return DefaultUserDataFolder(
            Environment.ProcessPath,
            Assembly.GetEntryAssembly()?.GetName().Name,
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            static folder =>
            {
                try
                {
                    Directory.CreateDirectory(folder);
                    return true;
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException)
                {
                    return false;
                }
            });
    }

    internal static string? DefaultUserDataFolder(string? processPath, string? appName, string localAppData,
        Func<string, bool> canCreateFolder)
    {
        if (string.IsNullOrEmpty(processPath) || string.IsNullOrEmpty(localAppData))
            return null;

        var exeName = Path.GetFileNameWithoutExtension(processPath);
        var isDotnetHost = string.Equals(exeName, "dotnet", StringComparison.OrdinalIgnoreCase);
        if (!isDotnetHost)
        {
            var nextToExe = Path.Combine(Path.GetDirectoryName(processPath) ?? string.Empty, exeName + ".WebView2");
            if (canCreateFolder(nextToExe))
                return null;
        }

        return Path.Combine(localAppData, string.IsNullOrEmpty(appName) ? exeName : appName, "WebView2");
    }

    public RustinoWindow SetMediaAutoplayEnabled(bool enabled)
    {
        SetCreationOnly(ref _mediaAutoplay, enabled, nameof(SetMediaAutoplayEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_media_autoplay(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetZoomHotkeysEnabled(bool enabled)
    {
        SetCreationOnly(ref _zoomHotkeys, enabled, nameof(SetZoomHotkeysEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_zoom_hotkeys(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow AddInitScript(string script)
    {
        lock (_lifecycleLock)
        {
            ThrowIfDisposed();
            if (_runStarted)
                throw CreationSettingStarted(nameof(AddInitScript));
            if (Native is { } handle && RustinoDllImports.rustino_add_init_script(handle, script) == 0)
                throw CreationSettingStarted(nameof(AddInitScript));
            _initScripts.Add(script);
        }
        return this;
    }

    // Serves requests for `scheme://...` URLs from .NET. Must be registered before WaitForClose().
    // Returning null from the handler produces a 404 response.
    public RustinoWindow RegisterCustomSchemeHandler(string scheme, NetCustomSchemeDelegate handler)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(scheme);
        ArgumentNullException.ThrowIfNull(handler);
        scheme = scheme.ToLowerInvariant();
        if (scheme == LocalFileScheme)
            throw new ArgumentException($"The {LocalFileScheme} scheme serves the local files passed to Load.", nameof(scheme));
        lock (_lifecycleLock)
        {
            ThrowIfDisposed();
            if (_runStarted)
                throw CreationSettingStarted(nameof(RegisterCustomSchemeHandler));
            if (Native is { } handle
                && !_customSchemes.ContainsKey(scheme)
                && RustinoDllImports.rustino_add_custom_scheme(handle, scheme) == 0)
                throw CreationSettingStarted(nameof(RegisterCustomSchemeHandler));
            _customSchemes[scheme] = handler;
        }
        return this;
    }

    public RustinoWindow SetAboutName(string name)
    {
        ThrowIfDisposed();
        _aboutName = name;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_name(handle, name);
        return this;
    }

    public RustinoWindow SetAboutVersion(string version)
    {
        ThrowIfDisposed();
        _aboutVersion = version;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_version(handle, version);
        return this;
    }

    public RustinoWindow SetAboutCopyright(string copyright)
    {
        ThrowIfDisposed();
        _aboutCopyright = copyright;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_copyright(handle, copyright);
        return this;
    }

    public RustinoWindow SetAboutWebsite(string website)
    {
        ThrowIfDisposed();
        _aboutWebsite = website;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_website(handle, website);
        return this;
    }

    public RustinoWindow SetAboutLicense(string license)
    {
        ThrowIfDisposed();
        _aboutLicense = license;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_license(handle, license);
        return this;
    }

    public RustinoWindow AddAboutAuthor(string author)
    {
        ThrowIfDisposed();
        _aboutAuthors.Add(author);
        if (Native is { } handle)
        {
            var authorsStr = string.Join("\n", _aboutAuthors);
            RustinoDllImports.rustino_set_about_authors(handle, authorsStr);
        }
        return this;
    }

    public RustinoWindow SetAboutComments(string comments)
    {
        ThrowIfDisposed();
        _aboutComments = comments;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_comments(handle, comments);
        return this;
    }
}
