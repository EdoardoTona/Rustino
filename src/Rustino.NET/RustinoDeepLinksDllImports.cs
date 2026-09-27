using System.Runtime.InteropServices;

namespace Rustino.NET;

internal static partial class RustinoExtDllImports
{
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_deliver_urls(
        IntPtr instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string jsonUrls);
}
