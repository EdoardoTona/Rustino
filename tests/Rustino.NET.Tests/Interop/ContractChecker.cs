using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;

namespace Rustino.NET.Tests.Interop;

internal enum ProblemKind { Declaration, Signature, Callback }

internal sealed record Problem(ProblemKind Kind, string Message)
{
    public override string ToString() => Message;
}

/// <summary>
/// Compares Rust FFI types with the .NET types declared for them.
///
/// Mapping table (Rust → accepted .NET declarations):
/// <list type="table">
/// <item><term>i8 u8 i16 u16 i32 u32 i64 u64 f32 f64</term><description>sbyte byte short ushort int uint long ulong float double, or an enum with that underlying type</description></item>
/// <item><term>isize usize</term><description>nint/IntPtr, nuint/UIntPtr</description></item>
/// <item><term>c_int c_uint c_short ...</term><description>as their fixed-size equivalents; c_char: sbyte or byte; c_long/c_ulong: CLong/CULong</description></item>
/// <item><term>bool (1 byte)</term><description>bool with [MarshalAs(U1 or I1)], byte, sbyte</description></item>
/// <item><term>i32</term><description>int or an enum with int as its underlying type; flags use 0/1, not .NET bool</description></item>
/// <item><term>*const c_char parameter</term><description>string with UTF-8 marshalling (MarshalAs LPUTF8Str, MarshalUsing Utf8StringMarshaller, LibraryImport StringMarshalling.Utf8), IntPtr/nint, byte*, sbyte*</description></item>
/// <item><term>*mut c_char, or any c_char pointer returned</term><description>IntPtr/nint, byte*, sbyte* only: a string returned by native is freed with rustino_free_string</description></item>
/// <item><term>*const T / *mut T (T primitive, pointer or #[repr(C)] struct)</term><description>IntPtr/nint, T*, T[] (parameters), ref T, in T (*const only), out T (*mut only)</description></item>
/// <item><term>*mut Opaque / *const c_void</term><description>IntPtr/nint, void*, any pointer, or a SafeHandle (parameters and returns)</description></item>
/// <item><term>extern "C" fn / Option&lt;extern "C" fn&gt;</term><description>a delegate with [UnmanagedFunctionPointer(Cdecl)] whose Invoke matches (checked recursively), delegate* unmanaged[Cdecl], or IntPtr/nint (unchecked)</description></item>
/// <item><term>#[repr(C)] struct by value</term><description>a .NET struct, whose layout is checked against abi-layout.json</description></item>
/// <item><term>() or no return type</term><description>void</description></item>
/// </list>
/// </summary>
internal sealed class ContractChecker(RustCrate crate, IReadOnlyCollection<Type> netTypes)
{
    /// <summary>Rust struct name → .NET struct types it was paired with in signatures, with where.</summary>
    public Dictionary<string, List<(Type Type, string Where)>> StructPairs { get; } = [];

    private static readonly Dictionary<string, Type[]> Primitives = new()
    {
        ["i8"] = [typeof(sbyte)], ["u8"] = [typeof(byte)],
        ["i16"] = [typeof(short)], ["u16"] = [typeof(ushort)],
        ["i32"] = [typeof(int)], ["u32"] = [typeof(uint)],
        ["i64"] = [typeof(long)], ["u64"] = [typeof(ulong)],
        ["isize"] = [typeof(nint)], ["usize"] = [typeof(nuint)],
        ["f32"] = [typeof(float)], ["f64"] = [typeof(double)],
        ["char"] = [typeof(uint)],
        ["c_schar"] = [typeof(sbyte)], ["c_uchar"] = [typeof(byte)],
        ["c_short"] = [typeof(short)], ["c_ushort"] = [typeof(ushort)],
        ["c_int"] = [typeof(int)], ["c_uint"] = [typeof(uint)],
        ["c_longlong"] = [typeof(long)], ["c_ulonglong"] = [typeof(ulong)],
        ["c_float"] = [typeof(float)], ["c_double"] = [typeof(double)],
        // Signedness of C char differs between targets
        ["c_char"] = [typeof(sbyte), typeof(byte)],
        // 32 bits on Windows, 64 on 64-bit Unix
        ["c_long"] = [typeof(CLong)], ["c_ulong"] = [typeof(CULong)],
        ["bool"] = [typeof(bool), typeof(byte), typeof(sbyte)],
    };

    private static bool IsPrimitive(RustType t, out string name)
    {
        name = t is RustPath { Args.Count: 0 } p ? p.Name : "";
        return Primitives.ContainsKey(name);
    }

    private static bool IsCChar(RustType t) => t is RustPath { Name: "c_char", Args.Count: 0 };

    private static bool IsPointerLike(Type t) =>
        t == typeof(IntPtr) || t == typeof(UIntPtr) || t.IsPointer;

    /// <summary>Checks one import against the Rust export it binds to.</summary>
    public List<Problem> CheckImport(NetImport import, RustExport export)
    {
        var problems = new List<Problem>();
        var where = $"{export.Name} ({import.Display} vs {export.Location})";

        // Calling convention on both sides
        if (export.Abi is not ("C" or "C-unwind"))
            problems.Add(new(ProblemKind.Declaration,
                $"{where}: Rust export uses the {(export.Abi is null ? "Rust ABI (no extern)" : $"extern \"{export.Abi}\" ABI")}, expected extern \"C\" to match CallingConvention.Cdecl"));
        var utf8ByDefault = false;
        if (import.Kind == ImportKind.DllImport)
        {
            var dll = import.DllImport!;
            if (dll.CallingConvention != CallingConvention.Cdecl)
                problems.Add(new(ProblemKind.Declaration,
                    $"{where}: [DllImport] calling convention is {dll.CallingConvention}, expected CallingConvention.Cdecl"));
            if (!dll.PreserveSig)
                problems.Add(new(ProblemKind.Declaration, $"{where}: [DllImport] has PreserveSig = false, which rewrites the signature (HRESULT)"));
        }
        else
        {
            var callConv = import.Method.GetCustomAttribute<UnmanagedCallConvAttribute>();
            if (callConv?.CallConvs?.Contains(typeof(CallConvCdecl)) != true)
                problems.Add(new(ProblemKind.Declaration,
                    $"{where}: [LibraryImport] needs [UnmanagedCallConv(CallConvs = new[] {{ typeof(CallConvCdecl) }})]"));
            utf8ByDefault = import.LibraryImport!.StringMarshalling == StringMarshalling.Utf8;
        }

        var netParams = import.Method.GetParameters();
        if (netParams.Length != export.Params.Count)
        {
            problems.Add(new(ProblemKind.Signature,
                $"{where}: .NET declares {netParams.Length} parameter(s) ({string.Join(", ", netParams.Select(p => $"{NetSlot.FromParameter(p, SlotKind.ImportParam)} {p.Name}"))}), " +
                $"Rust has {export.Params.Count} ({string.Join(", ", export.Params.Select(p => $"{p.Name}: {p.Type}"))})"));
        }
        else
        {
            for (var i = 0; i < netParams.Length; i++)
            {
                var slot = NetSlot.FromParameter(netParams[i], SlotKind.ImportParam, utf8ByDefault);
                Check(export.Params[i].Type, slot,
                    $"{where} param #{i} '{export.Params[i].Name}' (.NET '{netParams[i].Name}')", problems);
            }
        }
        Check(export.Return, NetSlot.FromParameter(import.Method.ReturnParameter, SlotKind.ImportReturn, utf8ByDefault),
            $"{where} return value", problems);
        return problems;
    }

    /// <summary>Checks a Rust type against a .NET slot, appending what does not match.</summary>
    public void Check(RustType rustType, NetSlot slot, string where, List<Problem> problems)
    {
        var kind = slot.Kind is SlotKind.CallbackParam or SlotKind.CallbackReturn ? ProblemKind.Callback : ProblemKind.Signature;
        void Fail(string expected, string? why = null) =>
            problems.Add(new(kind, $"{where}: Rust `{rustType}` expects {expected}, .NET declares `{slot}`{(why is null ? "" : $" ({why})")}"));

        var rust = crate.Resolve(rustType);
        switch (rust)
        {
            case RustUnit or RustNever:
                if (slot.Type != typeof(void)) Fail("void");
                return;
            case RustPointer pointer:
                CheckPointer(pointer, slot, Fail, where, problems);
                return;
            case RustFunction fn:
                CheckFunction(fn, slot, Fail, where, problems);
                return;
            case RustPath path when IsPrimitive(path, out var name):
                CheckPrimitive(name, slot, Fail);
                return;
            case RustPath path when crate.FindStruct(path.Name) is not null:
                if (slot.IsByRef || !IsNetStruct(slot.Type))
                    Fail($"the .NET struct mirroring {path.Name}, passed by value");
                else
                    RecordPair(path.Name, slot.Type, where);
                return;
            case RustPath path:
                problems.Add(new(kind, $"{where}: Rust `{rustType}` is not FFI-safe by value (`{path.Name}` is neither a primitive nor a #[repr(C)] struct)"));
                return;
            default:
                problems.Add(new(kind, $"{where}: Rust `{rust}` cannot cross the FFI boundary (not supported by the interop checks)"));
                return;
        }
    }

    private static bool IsNetStruct(Type t) => t.IsValueType && !t.IsPrimitive && !t.IsEnum && t != typeof(IntPtr) && t != typeof(UIntPtr);

    private void RecordPair(string rustStruct, Type netType, string where)
    {
        if (!StructPairs.TryGetValue(rustStruct, out var list)) StructPairs[rustStruct] = list = [];
        list.Add((netType, where));
    }

    private static void CheckPrimitive(string name, NetSlot slot, Action<string, string?> fail)
    {
        var expected = Primitives[name];
        var expectedText = string.Join(" or ", expected.Select(NetSlot.NetTypeName));
        if (slot.Type == typeof(void))
        {
            fail(expectedText, null);
            return;
        }
        if (slot.IsByRef)
        {
            fail(expectedText + " by value", "passed by reference in .NET");
            return;
        }
        var t = slot.Type;
        if (t == typeof(bool))
        {
            var marshal = slot.MarshalAs?.Value;
            if (name == "bool" || name == "u8" || name == "i8")
            {
                if (marshal is not (UnmanagedType.U1 or UnmanagedType.I1))
                    fail("bool with [MarshalAs(UnmanagedType.U1)]", "Rust bool is 1 byte, a .NET bool marshals as a 4-byte BOOL by default");
                return;
            }
            if (name is "i32" or "u32" or "c_int" or "c_uint")
            {
                fail(expectedText, "an i32 export uses an integer parameter, not a .NET bool");
                return;
            }
            fail(expectedText, null);
            return;
        }
        if (name == "bool")
        {
            if (t != typeof(byte) && t != typeof(sbyte)) fail("bool with [MarshalAs(UnmanagedType.U1)], byte or sbyte", null);
            return;
        }
        var underlying = t.IsEnum ? Enum.GetUnderlyingType(t) : t;
        if (!expected.Contains(underlying))
            fail(expectedText + (t.IsEnum ? "" : $" (or an enum : {expectedText})"), null);
    }

    private void CheckPointer(RustPointer pointer, NetSlot slot, Action<string, string?> fail, string where, List<Problem> problems)
    {
        var pointee = pointer.Pointee;
        var isReturn = slot.IsReturn;

        // C strings
        if (IsCChar(pointee))
        {
            const string raw = "IntPtr/nint, byte* or sbyte*";
            if (IsPointerLike(slot.Type) && !slot.IsByRef)
                return;
            if (slot.Type == typeof(string))
            {
                if (isReturn)
                    fail(raw, pointer.Mutable
                        ? "a *mut c_char returned by native must be released with rustino_free_string: declare IntPtr, never string"
                        : "the string marshaller would free memory owned by native: declare IntPtr");
                else if (pointer.Mutable && slot.Kind == SlotKind.ImportParam)
                    fail(raw, "native may write through or take ownership of a *mut c_char");
                else if (!slot.IsUtf8String)
                    fail("string with UTF-8 marshalling ([MarshalAs(UnmanagedType.LPUTF8Str)])",
                        "without it strings are marshalled as ANSI on Windows");
                return;
            }
            fail(isReturn || pointer.Mutable ? raw : "string with UTF-8 marshalling, " + raw, null);
            return;
        }

        var pointeeResolved = crate.Resolve(pointee);
        var isStruct = pointeeResolved is RustPath sp && crate.FindStruct(sp.Name) is not null;
        var isTyped = isStruct || IsPrimitive(pointeeResolved, out _) || pointeeResolved is RustPointer;

        if (!isTyped)
        {
            // Opaque handle (c_void, or a Rust type .NET never looks into)
            var handleOk = !slot.IsByRef && (IsPointerLike(slot.Type)
                || (typeof(SafeHandle).IsAssignableFrom(slot.Type) && slot.Kind is SlotKind.ImportParam or SlotKind.ImportReturn));
            if (!handleOk)
                fail("IntPtr/nint (an opaque pointer)",
                    slot.IsByRef ? "ref/out adds a level of indirection" : null);
            return;
        }

        var expected = $"IntPtr/nint, {Describe(pointeeResolved)}*, ref {Describe(pointeeResolved)}";
        if (!slot.IsByRef)
        {
            if (slot.Type == typeof(IntPtr) || slot.Type == typeof(UIntPtr))
                return;
            if (slot.Type.IsPointer)
            {
                CheckElement(pointeeResolved, slot.Type.GetElementType()!, slot, expected, fail, where, problems);
                return;
            }
            if (slot.Type.IsArray && slot.Kind == SlotKind.ImportParam)
            {
                CheckElement(pointeeResolved, slot.Type.GetElementType()!, slot, expected, fail, where, problems);
                return;
            }
            fail(expected, null);
            return;
        }

        if (!pointer.Mutable && slot.IsOut && !slot.IsIn)
        {
            fail($"ref or in {Describe(pointeeResolved)}", "native only reads through *const, out would not pass the value");
            return;
        }
        if (pointer.Mutable && slot.IsIn && !slot.IsOut)
        {
            fail($"ref or out {Describe(pointeeResolved)}", "native writes through *mut, in is read-only");
            return;
        }
        CheckElement(pointeeResolved, slot.ValueType, slot, expected, fail, where, problems);
    }

    // The value a typed pointer points to must match like a by-value parameter
    private void CheckElement(RustType pointee, Type element, NetSlot slot, string expected, Action<string, string?> fail,
        string where, List<Problem> problems)
    {
        if (element == typeof(void)) return;
        var inner = new List<Problem>();
        Check(pointee, NetSlot.Raw(element, SlotKind.StructField) with { MarshalAs = slot.MarshalAs }, where, inner);
        if (inner.Count > 0) fail(expected, $"pointee: .NET {NetSlot.NetTypeName(element)}");
    }

    private static string Describe(RustType t) => t switch
    {
        RustPath p when Primitives.TryGetValue(p.Name, out var types) => NetSlot.NetTypeName(types[0]),
        RustPath p => p.Name,
        RustPointer => "IntPtr",
        _ => t.ToString(),
    };

    private void CheckFunction(RustFunction fn, NetSlot slot, Action<string, string?> fail, string where, List<Problem> problems)
    {
        if (fn.Abi is not ("C" or "C-unwind"))
            problems.Add(new(ProblemKind.Callback,
                $"{where}: Rust function pointer `{fn}` uses the {(fn.Abi is null ? "Rust ABI" : $"\"{fn.Abi}\" ABI")}, expected extern \"C\""));

        const string expected = "a [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate, delegate* unmanaged[Cdecl]<...> or IntPtr";
        var t = slot.ValueType;
        if (slot.IsByRef)
        {
            fail(expected, "passed by reference");
            return;
        }
        if (t == typeof(IntPtr) || t.IsPointer)
        {
            // An untyped function pointer: check the delegate named like the Rust alias, if any
            if (fn.Alias is { } alias)
            {
                var named = netTypes.Where(x => typeof(Delegate).IsAssignableFrom(x) && x.Name == alias).ToList();
                if (named.Count == 0)
                    problems.Add(new(ProblemKind.Callback,
                        $"{where}: Rust callback type `{alias}` crosses as IntPtr but no .NET delegate named {alias} exists to check its signature against"));
                foreach (var d in named)
                    CheckDelegate(fn, d, $"{where} (delegate {NetSlot.NetTypeName(d)} for Rust {alias})", problems);
            }
            return;
        }
        if (typeof(Delegate).IsAssignableFrom(t))
        {
            CheckDelegate(fn, t, where, problems);
            return;
        }
        if (t.IsFunctionPointer)
        {
            CheckFunctionPointer(fn, slot, where, problems);
            return;
        }
        fail(expected, null);
    }

    public void CheckDelegate(RustFunction fn, Type delegateType, string where, List<Problem> problems)
    {
        var name = NetSlot.NetTypeName(delegateType);
        if (delegateType.IsGenericType)
        {
            problems.Add(new(ProblemKind.Callback, $"{where}: generic delegate {name} cannot be marshalled as a function pointer, declare a dedicated delegate type"));
            return;
        }
        var attr = delegateType.GetCustomAttribute<UnmanagedFunctionPointerAttribute>();
        if (attr is null)
            problems.Add(new(ProblemKind.Callback,
                $"{where}: delegate {name} has no [UnmanagedFunctionPointer(CallingConvention.Cdecl)] (the default is Winapi, stdcall on 32-bit Windows)"));
        else if (attr.CallingConvention != CallingConvention.Cdecl)
            problems.Add(new(ProblemKind.Callback,
                $"{where}: delegate {name} uses CallingConvention.{attr.CallingConvention}, expected Cdecl"));

        var invoke = delegateType.GetMethod("Invoke")!;
        var parameters = invoke.GetParameters();
        var inner = $"{where} -> delegate {name}";
        if (parameters.Length != fn.Params.Count)
        {
            problems.Add(new(ProblemKind.Callback,
                $"{inner}: .NET delegate has {parameters.Length} parameter(s) ({string.Join(", ", parameters.Select(p => $"{NetSlot.FromParameter(p, SlotKind.CallbackParam)} {p.Name}"))}), " +
                $"Rust `{fn}` has {fn.Params.Count}"));
        }
        else
        {
            for (var i = 0; i < parameters.Length; i++)
                Check(fn.Params[i], NetSlot.FromParameter(parameters[i], SlotKind.CallbackParam),
                    $"{inner} param #{i} '{parameters[i].Name}'", problems);
        }
        Check(fn.Return, NetSlot.FromParameter(invoke.ReturnParameter, SlotKind.CallbackReturn), $"{inner} return value", problems);
    }

    private void CheckFunctionPointer(RustFunction fn, NetSlot slot, string where, List<Problem> problems)
    {
        var t = slot.ModifiedType ?? slot.Type;
        if (!t.IsUnmanagedFunctionPointer)
        {
            problems.Add(new(ProblemKind.Callback, $"{where}: managed function pointers cannot be called from native code, use delegate* unmanaged[Cdecl]"));
            return;
        }
        var conventions = t.GetFunctionPointerCallingConventions();
        if (!conventions.Contains(typeof(CallConvCdecl)))
            problems.Add(new(ProblemKind.Callback,
                $"{where}: delegate* unmanaged has calling convention [{string.Join(", ", conventions.Select(c => c.Name))}], expected [Cdecl]"));
        var parameters = t.GetFunctionPointerParameterTypes();
        if (parameters.Length != fn.Params.Count)
        {
            problems.Add(new(ProblemKind.Callback, $"{where}: function pointer has {parameters.Length} parameter(s), Rust `{fn}` has {fn.Params.Count}"));
            return;
        }
        for (var i = 0; i < parameters.Length; i++)
            Check(fn.Params[i], NetSlot.Raw(parameters[i].UnderlyingSystemType, SlotKind.CallbackParam), $"{where} -> function pointer param #{i}", problems);
        Check(fn.Return, NetSlot.Raw(t.GetFunctionPointerReturnType().UnderlyingSystemType, SlotKind.CallbackReturn),
            $"{where} -> function pointer return value", problems);
    }

    public static string Report(string title, IEnumerable<object> problems)
    {
        var list = problems.Select(p => p.ToString()!).ToList();
        var sb = new StringBuilder();
        sb.Append(title).Append(" (").Append(list.Count).Append("):");
        foreach (var p in list) sb.Append("\n - ").Append(p);
        return sb.ToString();
    }
}
