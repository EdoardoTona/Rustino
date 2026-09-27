using System.Reflection;
using System.Runtime.InteropServices;
using Rustino.NET.Tests.Interop;

namespace Rustino.NET.Tests;

/// <summary>
/// The #[repr(C)] structs crossing the boundary: the Rust source, the layout snapshot written by
/// rustc (src/Rustino.Native/abi-layout.json) and the .NET structs must all agree.
/// </summary>
public class StructLayoutTests
{
    private static InteropContract Contract => InteropContract.Current;

    private static string SnapshotPath => Path.Combine(RepoPaths.NativeCrate, AbiSnapshot.FileName);

    private static string Normalize(string name) => name.Replace("_", "").ToLowerInvariant();

    private static void AssertNone(string title, List<object> problems)
    {
        if (problems.Count > 0) Assert.Fail(ContractChecker.Report(title, problems));
    }

    [Fact]
    public void SnapshotMatchesTheRustSource()
    {
        Assert.True(File.Exists(SnapshotPath), $"{SnapshotPath} is missing: {AbiSnapshot.UpdateHint}");
        var snapshot = AbiSnapshot.Load(SnapshotPath);
        var problems = new List<object>();

        foreach (var rust in Contract.Crate.Structs)
        {
            var entry = snapshot.Find(rust.Name);
            if (entry is null)
            {
                problems.Add($"#[repr(C)] struct {rust.Name} ({rust.Location}) is not in {AbiSnapshot.FileName}: add it to layouts() in src/abi_layout.rs and {AbiSnapshot.UpdateHint}");
                continue;
            }
            var rustFields = rust.Fields.Select(f => f.Name).ToList();
            var snapFields = entry.Fields.Select(f => f.Name).ToList();
            if (!rustFields.SequenceEqual(snapFields))
                problems.Add($"{rust.Name} ({rust.Location}): fields in source [{string.Join(", ", rustFields)}] differ from the snapshot [{string.Join(", ", snapFields)}]: {AbiSnapshot.UpdateHint}");
        }
        foreach (var entry in snapshot.Structs.Where(s => Contract.Crate.FindStruct(s.Name) is null))
            problems.Add($"{AbiSnapshot.FileName} has {entry.Name}, which is no longer a #[repr(C)] struct in the sources: {AbiSnapshot.UpdateHint}");

        AssertNone($"{AbiSnapshot.FileName} is out of date", problems);
    }

    [Fact]
    public void NetStructsMatchTheSnapshot()
    {
        Assert.True(File.Exists(SnapshotPath), $"{SnapshotPath} is missing: {AbiSnapshot.UpdateHint}");
        var snapshot = AbiSnapshot.Load(SnapshotPath);
        var problems = new List<object>();
        var pointerWidth = IntPtr.Size * 8;
        Assert.True(snapshot.PointerWidth == pointerWidth,
            $"{AbiSnapshot.FileName} pins a {snapshot.PointerWidth}-bit layout, this process is {pointerWidth}-bit: offsets cannot be compared");

        foreach (var (rustName, reason) in ContractExceptions.RustOnlyStructs)
            if (Contract.Crate.FindStruct(rustName) is null)
                problems.Add($"ContractExceptions.RustOnlyStructs lists {rustName} (\"{reason}\") but there is no such #[repr(C)] struct: remove the entry");

        foreach (var rust in Contract.Crate.Structs.Where(s => !ContractExceptions.RustOnlyStructs.ContainsKey(s.Name)))
        {
            var candidates = (Contract.Checker.StructPairs.GetValueOrDefault(rust.Name) ?? []).Select(p => p.Type).ToList();
            if (ContractExceptions.StructPairs.TryGetValue(rust.Name, out var netName))
            {
                var named = Contract.NetTypes.Where(t => t.Name == netName).ToList();
                if (named.Count == 0)
                    problems.Add($"ContractExceptions.StructPairs maps {rust.Name} to {netName}, which is not a type of Rustino.NET");
                candidates.AddRange(named);
            }
            candidates = candidates.Distinct().ToList();
            if (candidates.Count == 0)
            {
                problems.Add($"#[repr(C)] struct {rust.Name} ({rust.Location}) has no .NET counterpart: no import or callback passes it by value, ref or pointer. " +
                             "Map it in ContractExceptions.StructPairs, or list it in ContractExceptions.RustOnlyStructs if it never crosses the boundary");
                continue;
            }
            if (candidates.Count > 1)
            {
                var uses = Contract.Checker.StructPairs.GetValueOrDefault(rust.Name) ?? [];
                problems.Add($"#[repr(C)] struct {rust.Name} maps to several .NET types: {string.Join("; ", uses.Select(u => $"{NetSlot.NetTypeName(u.Type)} in {u.Where}"))}");
            }
            var entry = snapshot.Find(rust.Name);
            if (entry is null)
            {
                problems.Add($"#[repr(C)] struct {rust.Name} is not in {AbiSnapshot.FileName}: {AbiSnapshot.UpdateHint}");
                continue;
            }
            foreach (var net in candidates)
                CompareStruct(rust, entry, net, problems);
        }
        AssertNone(".NET structs whose layout differs from the Rust #[repr(C)] struct", problems);
    }

    private static void CompareStruct(RustStruct rust, AbiStruct entry, Type net, List<object> problems)
    {
        var where = $"{rust.Name} ({rust.Location}) vs {NetSlot.NetTypeName(net)}";
        var layout = net.StructLayoutAttribute?.Value;
        if (layout is not (LayoutKind.Sequential or LayoutKind.Explicit))
        {
            problems.Add($"{where}: [StructLayout] is {layout?.ToString() ?? "missing"}, expected LayoutKind.Sequential");
            return;
        }

        var fields = net.GetFields(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)
            .Select(f => (Field: f, Offset: (int)Marshal.OffsetOf(net, f.Name)))
            .OrderBy(f => f.Offset)
            .ToList();
        if (fields.Count != entry.Fields.Count)
        {
            problems.Add($"{where}: .NET has {fields.Count} fields [{string.Join(", ", fields.Select(f => f.Field.Name))}], " +
                         $"Rust has {entry.Fields.Count} [{string.Join(", ", entry.Fields.Select(f => f.Name))}]");
        }

        for (var i = 0; i < Math.Min(fields.Count, entry.Fields.Count); i++)
        {
            var (field, offset) = fields[i];
            var expected = entry.Fields[i];
            var fieldWhere = $"{where} field #{i} '{expected.Name}' (.NET '{field.Name}')";
            if (Normalize(field.Name) != Normalize(expected.Name))
                problems.Add($"{fieldWhere}: name differs (fields must be declared in the same order on both sides)");
            if (offset != expected.Offset)
                problems.Add($"{fieldWhere}: offset {offset} in .NET, {expected.Offset} in Rust");

            var rustField = rust.Fields.FirstOrDefault(f => f.Name == expected.Name);
            if (rustField is null) continue;
            var typeProblems = new List<Problem>();
            Contract.Checker.Check(rustField.Type, NetSlot.FromField(field), fieldWhere, typeProblems);
            problems.AddRange(typeProblems);
        }

        var size = Marshal.SizeOf(net);
        if (size != entry.Size)
            problems.Add($"{where}: Marshal.SizeOf is {size} bytes, Rust size_of is {entry.Size}");
    }
}
