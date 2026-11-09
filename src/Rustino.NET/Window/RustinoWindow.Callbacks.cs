using System.Collections.Concurrent;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Extensions.Logging;

namespace Rustino.NET;

// The events of the native window: its callbacks find their RustinoWindow and raise the .NET events.
public partial class RustinoWindow
{
    // --- Instance routing for callbacks ---
    private static readonly ConcurrentDictionary<IntPtr, RustinoWindow> Instances = new();

    // Static delegate instances pinned by static fields (prevents GC)
    private static readonly ClosingCallback ClosingCb = OnClosingNative;
    private static readonly VoidContextCallback ClosedCb = OnClosedNative;
    private static readonly SizeCallback ResizedCb = OnResizedNative;
    private static readonly PointCallback MovedCb = OnMovedNative;
    private static readonly IntCallback FocusCb = OnFocusChangedNative;
    private static readonly WebMessageCallback WebMsgCb = OnWebMessageNative;
    private static readonly PageLoadCallback PageLoadCb = OnPageLoadNative;
    private static readonly NavigationCallback NavCb = OnNavigationNative;
    private static readonly MenuItemCallback MenuItemCb = OnMenuItemClickedNative;
    private static readonly TrayIconCallback TrayCb = OnTrayIconClickedNative;
    private static readonly CustomSchemeCallback CustomSchemeCb = OnCustomSchemeNative;
    private static readonly LogCallback LogCb = OnLogMessageNative;

    // --- Logging delegate ---
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void LogCallback(IntPtr context, int level, IntPtr message);

    // --- Callback wiring ---

    private void RegisterCallbacks(RustinoWindowHandle handle)
    {
        Instances[handle.Key] = this;
        RustinoDllImports.rustino_set_callback_context(handle, handle.Key);
        RustinoDllImports.rustino_set_closing_handler(handle, ClosingCb);
        RustinoDllImports.rustino_set_closed_handler(handle, ClosedCb);
        RustinoDllImports.rustino_set_resized_handler(handle, ResizedCb);
        RustinoDllImports.rustino_set_moved_handler(handle, MovedCb);
        RustinoDllImports.rustino_set_focus_changed_handler(handle, FocusCb);
        RegisterExtCallbacks(handle);
        RegisterWebViewCallbacks(handle);
        RustinoDllImports.rustino_set_web_message_received_handler(handle, WebMsgCb);
        RustinoDllImports.rustino_set_page_load_handler(handle, PageLoadCb);
        RustinoDllImports.rustino_set_navigation_handler(handle, NavCb);
        RustinoDllImports.rustino_set_menu_event_handler(handle, MenuItemCb);
        RustinoDllImports.rustino_set_tray_icon_event_handler(handle, TrayCb);
        RustinoDllImports.rustino_set_custom_scheme_handler(handle, CustomSchemeCb);
    }

    // --- Static native callbacks ---

    private static int OnClosingNative(IntPtr ctx) => GuardNativeCallback(ctx, nameof(OnClosingNative), 0, w =>
    {
        if (w.WindowClosing is not { } handler) return 0;
        var args = new CancelEventArgs();
        handler.Invoke(w, args);
        return args.Cancel ? 1 : 0;
    });

    // A failing handler or observer doesn't keep the streams from completing
    private static void OnClosedNative(IntPtr ctx) => GuardNativeCallback(ctx, nameof(OnClosedNative), w =>
    {
        try { w.WindowClosed?.Invoke(w, EventArgs.Empty); }
        catch (Exception exception) { ReportCallbackException(w, nameof(OnClosedNative), exception); }
        try { w._windowClosedObs.Emit(EventArgs.Empty); }
        catch (Exception exception) { ReportCallbackException(w, nameof(OnClosedNative), exception); }
        CompleteAllObservables(w);
    });

    private static void OnResizedNative(IntPtr ctx, int width, int height) => GuardNativeCallback(ctx, nameof(OnResizedNative), w =>
    {
        w.SizeChanged?.Invoke(w, new SizeEventArgs(width, height));
        w._sizeChangedObs.Emit((width, height));
    });

    private static void OnMovedNative(IntPtr ctx, int x, int y) => GuardNativeCallback(ctx, nameof(OnMovedNative), w =>
    {
        w.LocationChanged?.Invoke(w, new PointEventArgs(x, y));
        w._locationChangedObs.Emit((x, y));
    });

    private static void OnFocusChangedNative(IntPtr ctx, int focused) => GuardNativeCallback(ctx, nameof(OnFocusChangedNative), w =>
    {
        var isFocused = focused != 0;
        w.FocusChanged?.Invoke(w, isFocused);
        w._focusChangedObs.Emit(isFocused);
    });

    private static void OnWebMessageNative(IntPtr ctx, IntPtr msgPtr, IntPtr sourceUrlPtr) => GuardNativeCallback(ctx, nameof(OnWebMessageNative), w =>
    {
        var msg = Marshal.PtrToStringUTF8(msgPtr);
        if (msg == null) return;
        w.WebMessageReceived?.Invoke(w, msg);
        w._webMessageObs.Emit(msg);

        var args = new WebMessageEventArgs(msg, Marshal.PtrToStringUTF8(sourceUrlPtr) ?? "");
        w.WebMessageReceivedWithSource?.Invoke(w, args);
        w._webMessageWithSourceObs.Emit(args);
    });

    private static void OnPageLoadNative(IntPtr ctx, int eventType, IntPtr urlPtr) => GuardNativeCallback(ctx, nameof(OnPageLoadNative), w =>
    {
        var url = Marshal.PtrToStringUTF8(urlPtr) ?? "";
        var args = new PageLoadEventArgs(eventType == 0, url);
        w.PageLoaded?.Invoke(w, args);
        w._pageLoadedObs.Emit(args);

        if (OperatingSystem.IsMacOS() && eventType == 1 && w._iconFile != null)
        {
            SetMacDockIcon(w._iconFile);
        }
    });

    private static int OnNavigationNative(IntPtr ctx, IntPtr urlPtr) => GuardNativeCallback(ctx, nameof(OnNavigationNative), 0, w =>
    {
        var url = Marshal.PtrToStringUTF8(urlPtr) ?? "";
        var args = new NavigationEventArgs(url);
        w.Navigating?.Invoke(w, args);
        w._navigatingObs.Emit(args);
        return args.Cancel ? 1 : 0;
    });

    private static void OnMenuItemClickedNative(IntPtr ctx, IntPtr idPtr, int isChecked) => GuardNativeCallback(ctx, nameof(OnMenuItemClickedNative), w =>
    {
        var id = Marshal.PtrToStringUTF8(idPtr);
        if (id == null) return;
        w.MenuItemClicked?.Invoke(w, id);
        w._menuItemClickedObs.Emit(id);

        if (isChecked < 0) return;
        var args = new MenuItemCheckedEventArgs(id, isChecked != 0);
        w.MenuItemCheckedChanged?.Invoke(w, args);
        w._menuItemCheckedChangedObs.Emit(args);
    });

    private static void OnTrayIconClickedNative(IntPtr ctx, int button, int x, int y) => GuardNativeCallback(ctx, nameof(OnTrayIconClickedNative), w =>
    {
        var args = new TrayIconClickedEventArgs((TrayMouseButton)button, x, y);
        w.TrayIconClicked?.Invoke(w, args);
        w._trayIconClickedObs.Emit(args);
    });

    private static void OnCustomSchemeNative(IntPtr ctx, IntPtr urlPtr, IntPtr response) => GuardNativeCallback(ctx, nameof(OnCustomSchemeNative), w =>
    {
        var url = Marshal.PtrToStringUTF8(urlPtr);
        if (url == null) return;
        var scheme = url.Split(':', 2)[0];
        if (!w._customSchemes.TryGetValue(scheme, out var handler)) return;

        using var content = handler(w, scheme, url, out var contentType);
        if (content == null) return;
        using var buffer = new MemoryStream();
        content.CopyTo(buffer);
        RustinoDllImports.rustino_set_scheme_response(response, buffer.GetBuffer(), (int)buffer.Length, contentType);
    });

    private static void OnLogMessageNative(IntPtr ctx, int level, IntPtr messagePtr)
    {
        RustinoWindow? window = null;
        try
        {
            if (ctx == IntPtr.Zero) return;
            var handle = GCHandle.FromIntPtr(ctx);
            if (handle.Target is not WeakReference<RustinoWindow> weak || !weak.TryGetTarget(out window)) return;
            if (window._logger == null) return;

            var message = Marshal.PtrToStringUTF8(messagePtr);
            if (string.IsNullOrEmpty(message)) return;

            var logLevel = level switch
            {
                0 => LogLevel.Trace,
                1 => LogLevel.Debug,
                2 => LogLevel.Information,
                3 => LogLevel.Warning,
                4 => LogLevel.Error,
                5 => LogLevel.Critical,
                _ => LogLevel.Information
            };

            window._logger.Log(logLevel, message);
        }
        catch (Exception exception)
        {
            if (window is not null)
                ReportCallbackException(window, nameof(OnLogMessageNative), exception);
        }
    }

    private static void GuardNativeCallback(IntPtr context, string callbackName, Action<RustinoWindow> callback)
    {
        GuardNativeCallback(context, callbackName, 0, window =>
        {
            callback(window);
            return 0;
        });
    }

    private static T GuardNativeCallback<T>(IntPtr context, string callbackName, T fallback, Func<RustinoWindow, T> callback)
    {
        RustinoWindow? window = null;
        try
        {
            if (!Instances.TryGetValue(context, out window)) return fallback;
            return callback(window);
        }
        catch (Exception exception)
        {
            if (window is not null)
                ReportCallbackException(window, callbackName, exception);
            return fallback;
        }
    }

    private static void ReportCallbackException(RustinoWindow window, string callbackName, Exception exception)
    {
        try { window._logger?.LogError(exception, "Unhandled exception in Rustino callback {CallbackName}.", callbackName); }
        catch (Exception) { }

        try { window.UnhandledCallbackException?.Invoke(window, new NativeCallbackExceptionEventArgs(callbackName, exception)); }
        catch (Exception reportingException)
        {
            try { window._logger?.LogError(reportingException, "Unhandled exception in Rustino callback exception handler."); }
            catch (Exception) { }
        }
    }

    // --- Observable completion ---

    // Every stream completes even when observers throw; their exceptions are reported
    private static void CompleteAllObservables(RustinoWindow w)
    {
        ICompletable[] observables =
        [
            w._sizeChangedObs,
            .. w.ExtObservables,
            .. w.WebViewObservables,
            w._locationChangedObs,
            w._focusChangedObs,
            w._webMessageObs,
            w._webMessageWithSourceObs,
            w._pageLoadedObs,
            w._navigatingObs,
            w._windowClosedObs,
            w._menuItemClickedObs,
            w._menuItemCheckedChangedObs,
            w._trayIconClickedObs,
        ];
        foreach (var observable in observables)
        {
            try { observable.Complete(); }
            catch (Exception exception) { ReportCallbackException(w, nameof(IObserver<object>.OnCompleted), exception); }
        }
    }
}
