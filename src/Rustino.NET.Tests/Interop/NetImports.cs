using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;

namespace Rustino.NET.Tests.Interop;

internal enum ImportKind { DllImport, LibraryImport }

/// <summary>A P/Invoke declaration bound to the native library.</summary>
internal sealed record NetImport(MethodInfo Method, ImportKind Kind, string EntryPoint)
{
    public string Display => $"{Method.DeclaringType?.Name}.{Method.Name}";

    public DllImportAttribute? DllImport => Method.GetCustomAttribute<DllImportAttribute>();
    public LibraryImportAttribute? LibraryImport => Method.GetCustomAttribute<LibraryImportAttribute>();

    /// <summary>
    /// Every [DllImport]/[LibraryImport] method of the assembly (any visibility) targeting
    /// <paramref name="library"/>. The stubs the LibraryImport generator emits are skipped: the
    /// declaration the developer wrote is the one checked.
    /// </summary>
    public static List<NetImport> Collect(Assembly assembly, string library) =>
        Collect(GetTypes(assembly), library);

    public static List<NetImport> Collect(IEnumerable<Type> types, string library)
    {
        const BindingFlags all = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static
                                 | BindingFlags.Instance | BindingFlags.DeclaredOnly;
        var imports = new List<NetImport>();
        foreach (var type in types)
        {
            foreach (var method in type.GetMethods(all))
            {
                if (method.Name.Contains('<') || method.IsDefined(typeof(CompilerGeneratedAttribute)))
                    continue;
                if (method.GetCustomAttribute<LibraryImportAttribute>() is { } li)
                {
                    if (li.LibraryName == library)
                        imports.Add(new NetImport(method, ImportKind.LibraryImport, li.EntryPoint ?? method.Name));
                }
                else if ((method.Attributes & MethodAttributes.PinvokeImpl) != 0
                         && method.GetCustomAttribute<DllImportAttribute>() is { } di
                         && di.Value == library)
                {
                    imports.Add(new NetImport(method, ImportKind.DllImport, di.EntryPoint ?? method.Name));
                }
            }
        }
        return imports;
    }

    public static Type[] GetTypes(Assembly assembly)
    {
        try
        {
            return assembly.GetTypes();
        }
        catch (ReflectionTypeLoadException e)
        {
            return e.Types.OfType<Type>().ToArray();
        }
    }
}

internal enum SlotKind { ImportParam, ImportReturn, CallbackParam, CallbackReturn, StructField }

/// <summary>A .NET value crossing the boundary (parameter, return value or struct field) with its marshalling info.</summary>
internal sealed record NetSlot(
    SlotKind Kind,
    Type Type,
    MarshalAsAttribute? MarshalAs,
    Type? MarshalUsing,
    bool Utf8ByDefault,
    bool IsIn,
    bool IsOut,
    Type? ModifiedType)
{
    public bool IsByRef => Type.IsByRef;
    public Type ValueType => Type.IsByRef ? Type.GetElementType()! : Type;
    public bool IsReturn => Kind is SlotKind.ImportReturn or SlotKind.CallbackReturn;

    public static NetSlot FromParameter(ParameterInfo p, SlotKind kind, bool utf8ByDefault = false)
    {
        Type? modified = null;
        try
        {
            modified = p.GetModifiedParameterType();
        }
        catch (NotSupportedException)
        {
        }
        return new NetSlot(
            kind,
            p.ParameterType,
            p.GetCustomAttribute<MarshalAsAttribute>(),
            p.GetCustomAttributes<MarshalUsingAttribute>().FirstOrDefault()?.NativeType,
            utf8ByDefault,
            p.IsIn || p.IsDefined(typeof(IsReadOnlyAttribute)),
            p.IsOut,
            modified);
    }

    public static NetSlot FromField(FieldInfo f) =>
        new(SlotKind.StructField, f.FieldType, f.GetCustomAttribute<MarshalAsAttribute>(), null, false, false, false,
            f.GetModifiedFieldType());

    /// <summary>Slot of a raw function pointer parameter (<c>delegate* unmanaged</c>): no marshalling.</summary>
    public static NetSlot Raw(Type type, SlotKind kind) => new(kind, type, null, null, false, false, false, type);

    public bool IsUtf8String =>
        MarshalAs?.Value == UnmanagedType.LPUTF8Str
        || MarshalUsing == typeof(Utf8StringMarshaller)
        || (MarshalAs is null && MarshalUsing is null && Utf8ByDefault);

    public override string ToString()
    {
        var prefix = "";
        if (MarshalAs is not null) prefix = $"[MarshalAs(UnmanagedType.{MarshalAs.Value})] ";
        else if (MarshalUsing is not null) prefix = $"[MarshalUsing(typeof({MarshalUsing.Name}))] ";
        var modifier = !IsByRef ? "" : IsOut && !IsIn ? "out " : IsIn && !IsOut ? "in " : "ref ";
        return prefix + modifier + NetTypeName(ValueType);
    }

    public static string NetTypeName(Type t)
    {
        if (t == typeof(void)) return "void";
        if (t == typeof(bool)) return "bool";
        if (t == typeof(byte)) return "byte";
        if (t == typeof(sbyte)) return "sbyte";
        if (t == typeof(short)) return "short";
        if (t == typeof(ushort)) return "ushort";
        if (t == typeof(int)) return "int";
        if (t == typeof(uint)) return "uint";
        if (t == typeof(long)) return "long";
        if (t == typeof(ulong)) return "ulong";
        if (t == typeof(float)) return "float";
        if (t == typeof(double)) return "double";
        if (t == typeof(char)) return "char";
        if (t == typeof(string)) return "string";
        if (t == typeof(IntPtr)) return "IntPtr";
        if (t == typeof(UIntPtr)) return "UIntPtr";
        if (t.IsPointer) return NetTypeName(t.GetElementType()!) + "*";
        if (t.IsArray) return NetTypeName(t.GetElementType()!) + "[]";
        if (t.IsFunctionPointer) return "delegate*<...>";
        return t.IsNested ? $"{t.DeclaringType!.Name}.{t.Name}" : t.Name;
    }
}
