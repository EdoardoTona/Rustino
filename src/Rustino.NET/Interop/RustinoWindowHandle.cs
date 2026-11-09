using System.Runtime.InteropServices;

namespace Rustino.NET;

/// <summary>
/// Owns the native window. Every P/Invoke that takes it holds a reference for the duration of the
/// call, so <see cref="RustinoWindow.Dispose"/> on another thread can't free the window while a call
/// uses it: the native destructor runs once the last call has returned.
/// </summary>
internal sealed class RustinoWindowHandle : SafeHandle
{
    // The context of the native log callback: valid until the native window is destroyed
    private GCHandle _logContext;

    public RustinoWindowHandle() : base(IntPtr.Zero, ownsHandle: true) { }

    internal RustinoWindowHandle(IntPtr handle, GCHandle logContext) : this()
    {
        SetHandle(handle);
        Key = handle;
        _logContext = logContext;
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    /// <summary>Identifies the window in the native callbacks; stays the same after disposal.</summary>
    internal IntPtr Key { get; }

    protected override bool ReleaseHandle()
    {
        RustinoDllImports.rustino_dtor(handle);
        if (_logContext.IsAllocated)
            _logContext.Free();
        return true;
    }
}
