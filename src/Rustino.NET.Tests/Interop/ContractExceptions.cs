namespace Rustino.NET.Tests.Interop;

/// <summary>
/// Intentional exceptions to the interop contract. Every entry needs a reason; entries that no
/// longer apply make the tests fail, so the lists stay short and current.
/// </summary>
internal static class ContractExceptions
{
    /// <summary>Rust exports that Rustino.NET deliberately does not import: export name → reason.</summary>
    public static readonly IReadOnlyDictionary<string, string> NotImported = new Dictionary<string, string>
    {
    };

    /// <summary>
    /// Imports of exports compiled only on some targets: export name → reason (which .NET code
    /// guards the call). The convention is to export every function on every platform and make
    /// it a no-op where unsupported, because calling a missing export throws EntryPointNotFoundException.
    /// </summary>
    public static readonly IReadOnlyDictionary<string, string> PlatformSpecificImports = new Dictionary<string, string>
    {
    };

    /// <summary>
    /// Rust #[repr(C)] struct → .NET struct type name, for structs whose .NET counterpart cannot be
    /// found from the signatures (e.g. passed as IntPtr). Pairs found in signatures need no entry.
    /// </summary>
    public static readonly IReadOnlyDictionary<string, string> StructPairs = new Dictionary<string, string>
    {
    };

    /// <summary>#[repr(C)] structs that never cross the boundary: struct name → reason.</summary>
    public static readonly IReadOnlyDictionary<string, string> RustOnlyStructs = new Dictionary<string, string>
    {
    };
}
