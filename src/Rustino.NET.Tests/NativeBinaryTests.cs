using System.Runtime.InteropServices;
using Rustino.NET.Tests.Interop;

namespace Rustino.NET.Tests;

/// <summary>
/// Binary-level check: the library built for the current platform exports what the sources
/// promise (respecting #[cfg] gating) and what .NET imports. Skipped when it is not built.
/// </summary>
public class NativeBinaryTests
{
    private static InteropContract Contract => InteropContract.Current;

    private static void WithLibrary(Action<IntPtr, string> check)
    {
        var path = NativeBinary.ExpectedPath;
        if (!NativeLibrary.TryLoad(path, out var handle))
            Assert.Fail($"{path} exists but cannot be loaded (built for another architecture?){NativeBinary.StalenessHint(path)}");
        try
        {
            check(handle, path);
        }
        finally
        {
            NativeLibrary.Free(handle);
        }
    }

    [NativeLibraryFact]
    public void EveryImportResolves() => WithLibrary((handle, path) =>
    {
        var target = RustTarget.Current;
        var features = Contract.Crate.DefaultFeatures;
        var problems = new List<string>();
        foreach (var import in Contract.Imports)
        {
            var variants = Contract.ExportsByName[import.EntryPoint].ToList();
            var compiledHere = variants.Count == 0 || variants.Any(e => InteropContract.SafeEnabled(e, target, features));
            if (!compiledHere) continue; // gated out on this platform: reported by the source-level tests
            if (!NativeLibrary.TryGetExport(handle, import.EntryPoint, out _))
                problems.Add($"{import.EntryPoint} ({import.Display}) is not exported by {Path.GetFileName(path)}");
        }
        if (problems.Count > 0)
            Assert.Fail(ContractChecker.Report($".NET imports missing from {path}", problems) + NativeBinary.StalenessHint(path));
    });

    [NativeLibraryFact]
    public void EveryExportCompiledForThisPlatformIsPresent() => WithLibrary((handle, path) =>
    {
        var target = RustTarget.Current;
        var features = Contract.Crate.DefaultFeatures;
        var problems = new List<string>();
        foreach (var group in Contract.ExportsByName)
        {
            var enabled = group.Where(e => InteropContract.SafeEnabled(e, target, features)).ToList();
            var found = NativeLibrary.TryGetExport(handle, group.Key, out _);
            if (enabled.Count > 0 && !found)
                problems.Add($"{group.Key} ({enabled[0].Location}) should be compiled for {target.Rid} but is not exported by {Path.GetFileName(path)}");
        }
        if (problems.Count > 0)
            Assert.Fail(ContractChecker.Report($"Rust exports missing from {path}", problems) + NativeBinary.StalenessHint(path));
    });
}
