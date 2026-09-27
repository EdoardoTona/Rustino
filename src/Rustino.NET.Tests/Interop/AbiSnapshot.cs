using System.Text.Json;
using System.Text.Json.Serialization;

namespace Rustino.NET.Tests.Interop;

/// <summary>
/// src/Rustino.Native/abi-layout.json: offsets and sizes of the #[repr(C)] structs as computed
/// by rustc (written by the abi_layout Rust test).
/// </summary>
internal sealed record AbiSnapshot(
    [property: JsonPropertyName("pointer_width")] int PointerWidth,
    [property: JsonPropertyName("structs")] IReadOnlyList<AbiStruct> Structs)
{
    public const string FileName = "abi-layout.json";
    public const string UpdateHint = "regenerate it with `UPDATE_ABI_SNAPSHOT=1 cargo test --release abi_layout` in src/Rustino.Native";

    public static AbiSnapshot Load(string path) =>
        JsonSerializer.Deserialize<AbiSnapshot>(File.ReadAllText(path))
        ?? throw new InvalidDataException($"{path} is empty");

    public AbiStruct? Find(string name) => Structs.FirstOrDefault(s => s.Name == name);
}

internal sealed record AbiStruct(
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("size")] int Size,
    [property: JsonPropertyName("align")] int Align,
    [property: JsonPropertyName("fields")] IReadOnlyList<AbiField> Fields);

internal sealed record AbiField(
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("offset")] int Offset,
    [property: JsonPropertyName("size")] int Size,
    [property: JsonPropertyName("align")] int Align);
