using System.Runtime.InteropServices;

namespace Rustino.NET;

internal static partial class RustinoDllImports
{
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_safe_storage_available([MarshalAs(UnmanagedType.LPUTF8Str)] string appId);

    // result: released with rustino_safe_storage_free
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_safe_storage_encrypt(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string appId,
        in byte data, nuint length, out IntPtr result, out nuint resultLength);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_safe_storage_decrypt(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string appId,
        in byte data, nuint length, out IntPtr result, out nuint resultLength);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_safe_storage_reset([MarshalAs(UnmanagedType.LPUTF8Str)] string appId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_safe_storage_free(IntPtr data, nuint length);
}
