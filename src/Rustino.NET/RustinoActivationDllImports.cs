using System.Runtime.InteropServices;

namespace Rustino.NET;

internal static partial class RustinoDllImports
{
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_activate(
        IntPtr instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? activationToken);
}
