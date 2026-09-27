using System.Text.Json;

namespace Rustino.NET;

public partial class RustinoWindow
{
    private HashSet<string>? _deepLinkSchemes;
    private int _deepLinksAttached;

    /// <summary>
    /// Queues URL arguments from the current launch and later single-instance launches through
    /// <see cref="UrlsOpened"/>. Attach handlers to <see cref="UrlsOpened"/> before calling this method.
    /// </summary>
    public RustinoWindow AttachDeepLinks(RustinoSingleInstance singleInstance, params string[] allowedSchemes)
    {
        ArgumentNullException.ThrowIfNull(singleInstance);
        ThrowIfDisposed();
        if (!singleInstance.IsPrimary)
            throw new InvalidOperationException("Deep links can only be attached by the primary process.");

        var normalized = RustinoDeepLinks.NormalizeSchemes(allowedSchemes);
        var schemes = new HashSet<string>(normalized, StringComparer.OrdinalIgnoreCase);
        lock (_lifecycleLock)
        {
            ThrowIfDisposed();
            EnsureNativeLocked();
        }
        if (Interlocked.CompareExchange(ref _deepLinksAttached, 1, 0) != 0)
            throw new InvalidOperationException("Deep links have already been attached to this window.");
        Volatile.Write(ref _deepLinkSchemes, schemes);

        var deliveryGate = new object();
        var weakWindow = new WeakReference<RustinoWindow>(this);
        EventHandler<SecondInstanceEventArgs> forward = (_, e) =>
        {
            if (!weakWindow.TryGetTarget(out var window)) return;
            var urls = RustinoDeepLinks.Filter(e.Args, schemes);
            if (urls.Length == 0) return;
            lock (deliveryGate) window.DeliverDeepLinks(urls);
        };

        lock (deliveryGate)
        {
            singleInstance.SecondInstanceStarted += forward;
            var currentUrls = RustinoDeepLinks.Filter(Environment.GetCommandLineArgs().Skip(1), schemes);
            if (currentUrls.Length > 0)
                DeliverDeepLinks(currentUrls);
        }
        return this;
    }

    internal string[] FilterUrlsOpened(string[] urls)
    {
        var schemes = Volatile.Read(ref _deepLinkSchemes);
        if (schemes is null) return urls;
        var allowed = new HashSet<string>(RustinoDeepLinks.Filter(urls, schemes), StringComparer.Ordinal);
        return urls.Where(url => allowed.Contains(url) || IsFileUrl(url)).Take(128).ToArray();
    }

    private static bool IsFileUrl(string value) =>
        Uri.TryCreate(value, UriKind.Absolute, out var uri) && uri.Scheme.Equals("file", StringComparison.OrdinalIgnoreCase);

    private void DeliverDeepLinks(string[] urls)
    {
        var json = JsonSerializer.Serialize(urls);
        lock (_lifecycleLock)
        {
            if (Volatile.Read(ref _disposed) != 0 || _nativeHandle == IntPtr.Zero)
                return;
            RustinoExtDllImports.rustino_deliver_urls(_nativeHandle, json);
        }
    }
}
