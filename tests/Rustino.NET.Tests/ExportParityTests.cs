using Rustino.NET.Tests.Interop;

namespace Rustino.NET.Tests;

/// <summary>
/// Source-level parity between the Rust C exports (src/Rustino.Native/src/**/*.rs) and the
/// P/Invoke declarations of Rustino.NET: names, calling convention, parameter and return types,
/// callbacks. Nothing is listed by hand, so new exports and imports are checked automatically.
/// </summary>
public class ExportParityTests
{
    private static InteropContract Contract => InteropContract.Current;

    private static void AssertNone(string title, IEnumerable<object> problems)
    {
        var list = problems.ToList();
        if (list.Count > 0) Assert.Fail(ContractChecker.Report(title, list));
    }

    [Fact]
    public void RustSourcesAreFullyUnderstood()
    {
        AssertNone("The Rust sources contain FFI items the interop checks cannot read", Contract.ParseProblems);
        Assert.True(Contract.Exports.Count > 0, $"No #[no_mangle] exports found in {RepoPaths.NativeCrate}/src");
        Assert.True(Contract.Imports.Count > 0, $"No [DllImport(\"{Contract.LibraryName}\")] methods found in {Contract.NetAssembly.GetName().Name}");
    }

    [Fact]
    public void EveryImportHasARustExport()
    {
        var missing = Contract.Imports
            .Where(i => !Contract.ExportsByName.Contains(i.EntryPoint))
            .Select(i => Contract.Crate.Exports.Any(e => e.Name == i.EntryPoint)
                ? $"{i.EntryPoint} ({i.Display}): the Rust export exists but is never compiled into the library (cfg: {string.Join(" && ", Contract.Crate.Exports.First(e => e.Name == i.EntryPoint).Cfgs)})"
                : $"{i.EntryPoint} ({i.Display}): no #[no_mangle] extern \"C\" fn {i.EntryPoint} in src/Rustino.Native/src")
            .Cast<object>();
        AssertNone(".NET imports without a Rust export (EntryPointNotFoundException at runtime)", missing);
    }

    [Fact]
    public void EveryRustExportIsImported()
    {
        var imported = Contract.Imports.Select(i => i.EntryPoint).ToHashSet();
        var problems = new List<object>();
        foreach (var name in Contract.ExportsByName.Select(g => g.Key).Order())
        {
            if (!imported.Contains(name) && !ContractExceptions.NotImported.ContainsKey(name))
                problems.Add($"{name} ({Contract.ExportsByName[name].First().Location}) is not imported by Rustino.NET " +
                             $"(declare it in a *DllImports class, or add it to ContractExceptions.NotImported with the reason)");
        }
        foreach (var (name, reason) in ContractExceptions.NotImported)
        {
            if (!Contract.ExportsByName.Contains(name))
                problems.Add($"ContractExceptions.NotImported lists {name} (\"{reason}\") but there is no such export: remove the entry");
            else if (imported.Contains(name))
                problems.Add($"ContractExceptions.NotImported lists {name} (\"{reason}\") but it is imported: remove the entry");
        }
        AssertNone("Rust exports not imported by .NET", problems);
    }

    [Fact]
    public void ImportsAndExportsUseTheCdeclCallingConvention() =>
        AssertNone("Calling convention mismatches",
            Contract.ImportProblems.Where(p => p.Kind == ProblemKind.Declaration));

    [Fact]
    public void ImportSignaturesMatchExports() =>
        AssertNone("Parameter or return types that differ between Rust and .NET",
            Contract.ImportProblems.Where(p => p.Kind == ProblemKind.Signature));

    [Fact]
    public void CallbackDelegatesMatchRustFunctionPointers() =>
        AssertNone("Callbacks whose .NET delegate does not match the Rust function pointer type",
            Contract.ImportProblems.Where(p => p.Kind == ProblemKind.Callback));

    [Fact]
    public void CallbackTypeAliasesMatchTheirDelegates()
    {
        // Rust `type X = extern "C" fn(..)` aliases and .NET delegates share names: check each pair,
        // including those that only cross the boundary as an IntPtr (e.g. struct fields)
        var problems = new List<Problem>();
        foreach (var (name, aliases) in Contract.Crate.Aliases)
        {
            if (aliases.Count != 1) continue;
            if (Contract.Crate.Resolve(new RustPath(name, [])) is not RustFunction fn) continue;
            foreach (var d in Contract.NetTypes.Where(t => typeof(Delegate).IsAssignableFrom(t) && t.Name == name))
                Contract.Checker.CheckDelegate(fn, d, $"{name} ({aliases[0].File}:{aliases[0].Line} vs delegate {NetSlot.NetTypeName(d)})", problems);
        }
        AssertNone("Rust callback type aliases that differ from the .NET delegate of the same name", problems);
    }

    [Fact]
    public void ImportedExportsAreCompiledOnEverySupportedPlatform()
    {
        var problems = new List<object>();
        foreach (var name in Contract.Imports.Select(i => i.EntryPoint).Distinct().Where(Contract.ExportsByName.Contains))
        {
            var missingOn = Contract.TargetsWithout(name);
            if (missingOn.Count > 0 && !ContractExceptions.PlatformSpecificImports.ContainsKey(name))
            {
                var export = Contract.ExportsByName[name].First();
                problems.Add($"{name} ({export.Location}, cfg: {string.Join(" && ", export.Cfgs)}) is not compiled for " +
                             $"{string.Join(", ", missingOn.Select(t => t.Rid))} but .NET imports it: export it everywhere (no-op where unsupported), " +
                             "or guard the .NET calls and add it to ContractExceptions.PlatformSpecificImports");
            }
        }
        foreach (var (name, reason) in ContractExceptions.PlatformSpecificImports)
        {
            if (!Contract.ExportsByName.Contains(name) || Contract.TargetsWithout(name).Count == 0)
                problems.Add($"ContractExceptions.PlatformSpecificImports lists {name} (\"{reason}\") but it is compiled on every platform: remove the entry");
        }
        AssertNone("Platform-gated exports imported by .NET", problems);
    }
}
