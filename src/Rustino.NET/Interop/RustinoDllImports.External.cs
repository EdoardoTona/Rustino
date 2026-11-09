using System.Runtime.InteropServices;

namespace Rustino.NET;

internal static partial class RustinoDllImports
{
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_open_external(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string url);
}
