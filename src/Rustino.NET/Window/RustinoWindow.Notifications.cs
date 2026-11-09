namespace Rustino.NET;

// System notifications: static, no window required.
public partial class RustinoWindow
{
    public static bool ShowNotification(string title, string body, string? iconPath = null, string? appId = null)
    {
        NativeLibraryResolver.EnsureRegistered();
        return RustinoDllImports.rustino_show_notification(title, body, iconPath, appId) != 0;
    }

    // Windows drops toasts sent with an appId it cannot resolve, and unpackaged apps
    // (plain .exe, dotnet tools) are not registered anywhere: call this once at startup
    // before passing that appId to ShowNotification. appId must be a valid AppUserModelID
    // (e.g. "Company.Product": at most 128 characters, no spaces or backslashes), otherwise
    // this returns false on every platform. The icon must be an image file (.png/.ico)
    // that stays at iconPath; omitting it removes a previously registered icon.
    // Nothing is written on macOS and Linux.
    public static bool RegisterNotificationAppId(string appId, string displayName, string? iconPath = null)
    {
        NativeLibraryResolver.EnsureRegistered();
        return RustinoDllImports.rustino_register_notification_app_id(appId, displayName, iconPath) != 0;
    }

    public static bool ShowNotification(string title, string body, Stream icon, string? appId = null)
    {
        var tempPath = Path.Combine(Path.GetTempPath(), $"rustino_notify_{Guid.NewGuid():N}.png");
        try
        {
            using (var fs = File.Create(tempPath))
                icon.CopyTo(fs);
            return ShowNotification(title, body, tempPath, appId);
        }
        finally
        {
            try { File.Delete(tempPath); } catch { }
        }
    }
}
