using System.Runtime.InteropServices;

namespace Rustino.NET;

internal static partial class RustinoDllImports
{
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_autostart_status([MarshalAs(UnmanagedType.LPUTF8Str)] string appId);

    // command: the program and its arguments, separated by '\n'
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_autostart_enable(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string appId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? displayName,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string command);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_autostart_disable([MarshalAs(UnmanagedType.LPUTF8Str)] string appId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_autostart_open_settings();
}
