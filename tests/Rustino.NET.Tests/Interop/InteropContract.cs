using System.Reflection;

namespace Rustino.NET.Tests.Interop;

/// <summary>
/// Both sides of the Rustino interop contract, read once per test run: the Rust exports parsed
/// from src/Rustino.Native/src and the P/Invoke declarations of Rustino.NET found by reflection.
/// </summary>
internal sealed class InteropContract
{
    private static readonly Lazy<InteropContract> CurrentLazy = new(() => new InteropContract());

    public static InteropContract Current => CurrentLazy.Value;

    public RustCrate Crate { get; }
    public Assembly NetAssembly { get; }
    public IReadOnlyList<Type> NetTypes { get; }
    public string LibraryName { get; }
    public IReadOnlyList<NetImport> Imports { get; }
    public ContractChecker Checker { get; }

    /// <summary>Exports built on at least one supported target with some feature set (cfg(test) helpers excluded).</summary>
    public IReadOnlyList<RustExport> Exports { get; }
    public ILookup<string, RustExport> ExportsByName { get; }

    /// <summary>Problems of the parse itself (unsupported constructs, cfg that cannot be evaluated).</summary>
    public List<string> ParseProblems { get; } = [];

    /// <summary>Signature, declaration and callback problems of every import checked against its export(s).</summary>
    public List<Problem> ImportProblems { get; } = [];

    private InteropContract()
    {
        Crate = RustCrate.Load(RepoPaths.NativeCrate);
        ParseProblems.AddRange(Crate.Problems);

        NetAssembly = typeof(RustinoWindow).Assembly;
        NetTypes = NetImport.GetTypes(NetAssembly);
        LibraryName = (string)NetAssembly.GetType("Rustino.NET.NativeLibraryResolver", throwOnError: true)!
            .GetField("LibName", BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.Static)!
            .GetRawConstantValue()!;
        Imports = NetImport.Collect(NetAssembly, LibraryName);
        Checker = new ContractChecker(Crate, NetTypes.ToList());

        var exports = new List<RustExport>();
        foreach (var export in Crate.Exports)
        {
            try
            {
                if (RustTarget.Supported.Any(t => export.IsEnabledOn(t, Crate.DefaultFeatures) || export.IsEnabledOn(t, Crate.AllFeatures)))
                    exports.Add(export);
            }
            catch (Exception e) when (e is NotSupportedException or FormatException)
            {
                ParseProblems.Add($"{export.Name} ({export.Location}): {e.Message}");
                exports.Add(export);
            }
        }
        Exports = exports;
        ExportsByName = exports.ToLookup(e => e.Name);

        foreach (var import in Imports)
            foreach (var export in ExportsByName[import.EntryPoint])
                ImportProblems.AddRange(Checker.CheckImport(import, export));
    }

    /// <summary>Supported targets on which no variant of the export is compiled (default features).</summary>
    public IReadOnlyList<RustTarget> TargetsWithout(string exportName) =>
        RustTarget.Supported
            .Where(t => !ExportsByName[exportName].Any(e => SafeEnabled(e, t, Crate.DefaultFeatures)))
            .ToList();

    public static bool SafeEnabled(RustExport export, RustTarget target, IReadOnlySet<string> features)
    {
        try
        {
            return export.IsEnabledOn(target, features);
        }
        catch (Exception e) when (e is NotSupportedException or FormatException)
        {
            return true;
        }
    }
}
