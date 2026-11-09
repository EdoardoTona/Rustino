using System.Runtime.InteropServices;

namespace Rustino.NET;

// The identity of the app for the system: AppUserModelID on Windows, Dock icon on macOS.
public partial class RustinoWindow
{
    [DllImport("/usr/lib/libobjc.A.dylib")]
    private static extern IntPtr objc_getClass(string name);

    [DllImport("/usr/lib/libobjc.A.dylib")]
    private static extern IntPtr sel_registerName(string name);

    [DllImport("/usr/lib/libobjc.A.dylib", EntryPoint = "objc_msgSend")]
    private static extern IntPtr objc_msgSend(IntPtr receiver, IntPtr selector);

    [DllImport("/usr/lib/libobjc.A.dylib", EntryPoint = "objc_msgSend")]
    private static extern IntPtr objc_msgSend(IntPtr receiver, IntPtr selector, IntPtr arg);

    [DllImport("shell32.dll", SetLastError = true)]
    private static extern void SetCurrentProcessExplicitAppUserModelID([MarshalAs(UnmanagedType.LPWStr)] string appId);

    private string? _applicationId;

    public RustinoWindow SetApplicationId(string applicationId)
    {
        ThrowIfDisposed();
        _applicationId = applicationId;
        if (OperatingSystem.IsWindows() && !IsDotnetTool())
        {
            try
            {
                SetCurrentProcessExplicitAppUserModelID(applicationId);
            }
            catch { }
        }
        return this;
    }

    private static IntPtr CreateNSString(string str)
    {
        IntPtr nsStringClass = objc_getClass("NSString");
        IntPtr stringWithUTF8StringSel = sel_registerName("stringWithUTF8String:");
        IntPtr utf8Ptr = Marshal.StringToCoTaskMemUTF8(str);
        try
        {
            return objc_msgSend(nsStringClass, stringWithUTF8StringSel, utf8Ptr);
        }
        finally
        {
            Marshal.FreeCoTaskMem(utf8Ptr);
        }
    }

    [DllImport("libSystem.dylib")]
    private static extern IntPtr dlopen(string path, int mode);

    // The window icon is the Dock icon of an app that has none of its own (e.g. `dotnet app.dll`).
    // An app bundle keeps the icon of its Info.plist, which macOS draws in the style of the
    // system: an image set here would replace it, without that style, while the app runs.
    private static void SetMacDockIcon(string iconPath)
    {
        try
        {
            if (OperatingSystem.IsMacOS())
            {
                dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", 1);
            }

            if (MacBundleHasIcon())
                return;

            IntPtr nsImageClass = objc_getClass("NSImage");
            if (nsImageClass == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to load NSImage class in SetMacDockIcon.");
                return;
            }

            IntPtr nsStringPath = CreateNSString(iconPath);
            if (nsStringPath == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to create NSString for path in SetMacDockIcon.");
                return;
            }

            IntPtr allocSel = sel_registerName("alloc");
            IntPtr nsImageAllocated = objc_msgSend(nsImageClass, allocSel);
            if (nsImageAllocated == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to allocate NSImage in SetMacDockIcon.");
                return;
            }

            IntPtr initSel = sel_registerName("initWithContentsOfFile:");
            IntPtr nsImage = objc_msgSend(nsImageAllocated, initSel, nsStringPath);
            if (nsImage == IntPtr.Zero)
            {
                Console.Error.WriteLine($"Failed to init NSImage from path in SetMacDockIcon: {iconPath}");
                return;
            }

            IntPtr nsAppClass = objc_getClass("NSApplication");
            if (nsAppClass == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to load NSApplication class in SetMacDockIcon.");
                return;
            }

            IntPtr sharedAppSel = sel_registerName("sharedApplication");
            IntPtr nsApp = objc_msgSend(nsAppClass, sharedAppSel);
            if (nsApp == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to get NSApplication instance in SetMacDockIcon.");
                return;
            }

            IntPtr setIconSel = sel_registerName("setApplicationIconImage:");
            objc_msgSend(nsApp, setIconSel, nsImage);

            IntPtr dockTileSel = sel_registerName("dockTile");
            IntPtr dockTile = objc_msgSend(nsApp, dockTileSel);
            if (dockTile != IntPtr.Zero)
            {
                IntPtr displaySel = sel_registerName("display");
                objc_msgSend(dockTile, displaySel);
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Failed to set macOS Dock icon: {ex}");
        }
    }

    private static bool MacBundleHasIcon()
    {
        IntPtr bundleClass = objc_getClass("NSBundle");
        if (bundleClass == IntPtr.Zero)
            return false;
        IntPtr bundle = objc_msgSend(bundleClass, sel_registerName("mainBundle"));
        if (bundle == IntPtr.Zero)
            return false;
        IntPtr infoValueSel = sel_registerName("objectForInfoDictionaryKey:");
        return objc_msgSend(bundle, infoValueSel, CreateNSString("CFBundleIconFile")) != IntPtr.Zero
            || objc_msgSend(bundle, infoValueSel, CreateNSString("CFBundleIconName")) != IntPtr.Zero;
    }

    private static bool IsDotnetTool()
    {
        var processPath = Environment.ProcessPath ?? string.Empty;
        if (processPath.Contains(".dotnet") && (processPath.Contains("tools") || processPath.Contains("store")))
            return true;

        var argv0 = Environment.GetCommandLineArgs().FirstOrDefault() ?? string.Empty;
        if (argv0.Contains(".dotnet") && (argv0.Contains("tools") || argv0.Contains("store")))
            return true;

        return false;
    }
}
