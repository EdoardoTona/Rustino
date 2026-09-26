using System.ComponentModel;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Rustino.NET;

public enum ScrollBarStyle
{
    /// <summary>The browser's scroll bars.</summary>
    Default = 0,
    /// <summary>The overlay scroll bars of Windows 11, shown while scrolling.</summary>
    FluentOverlay = 1,
}

public enum FileDropEventType
{
    Enter,
    Over,
    Drop,
    Leave,
}

public class FileDropEventArgs(FileDropEventType type, string[] paths, int x, int y) : EventArgs
{
    public FileDropEventType Type { get; } = type;

    /// <summary>Full paths of the dragged files: set with <c>Enter</c> and <c>Drop</c>, empty otherwise.</summary>
    public string[] Paths { get; } = paths;

    /// <summary>Position in logical (CSS) pixels from the top-left corner of the page; 0 with <c>Leave</c>.</summary>
    public int X { get; } = x;

    public int Y { get; } = y;
}

public class DownloadStartingEventArgs(string url, string suggestedPath) : CancelEventArgs
{
    public string Url { get; } = url;

    /// <summary>Where a browser would save the file: the Downloads folder, with a name that doesn't replace a file.</summary>
    public string SuggestedPath { get; } = suggestedPath;

    /// <summary>
    /// Where to save the file, replacing an existing one. When null (the default) the user chooses it in the native
    /// save dialog, which starts at <see cref="SuggestedPath"/>.
    /// </summary>
    public string? DestinationPath { get; set; }
}

public class DownloadCompletedEventArgs(string url, string? path) : EventArgs
{
    public string Url { get; } = url;

    /// <summary>Where the file was saved; null when the download failed or the user canceled the save dialog.</summary>
    public string? Path { get; } = path;

    public bool Success => Path != null;
}

[JsonConverter(typeof(JsonStringEnumConverter<CookieSameSite>))]
public enum CookieSameSite
{
    None,
    Lax,
    Strict,
}

public class RustinoCookie
{
    public RustinoCookie()
    {
    }

    public RustinoCookie(string name, string value)
    {
        Name = name;
        Value = value;
    }

    [JsonPropertyName("name")]
    public string Name { get; set; } = "";

    [JsonPropertyName("value")]
    public string Value { get; set; } = "";

    /// <summary>With a leading dot the cookie also goes to the subdomains.</summary>
    [JsonPropertyName("domain")]
    public string? Domain { get; set; }

    [JsonPropertyName("path")]
    public string? Path { get; set; }

    /// <summary>Null for session cookies.</summary>
    [JsonPropertyName("expires")]
    [JsonConverter(typeof(UnixSecondsConverter))]
    public DateTimeOffset? Expires { get; set; }

    [JsonPropertyName("secure")]
    public bool Secure { get; set; }

    [JsonPropertyName("httpOnly")]
    public bool HttpOnly { get; set; }

    /// <summary>Null when the cookie doesn't set it.</summary>
    [JsonPropertyName("sameSite")]
    public CookieSameSite? SameSite { get; set; }

    public override string ToString() => $"{Name}={Value} ({Domain}{Path})";
}

internal sealed class UnixSecondsConverter : JsonConverter<DateTimeOffset?>
{
    public override DateTimeOffset? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        reader.TokenType == JsonTokenType.Null ? null : DateTimeOffset.FromUnixTimeSeconds(reader.GetInt64());

    public override void Write(Utf8JsonWriter writer, DateTimeOffset? value, JsonSerializerOptions options)
    {
        if (value is { } time)
            writer.WriteNumberValue(time.ToUnixTimeSeconds());
        else
            writer.WriteNullValue();
    }
}

[JsonSerializable(typeof(RustinoCookie))]
[JsonSerializable(typeof(RustinoCookie[]))]
internal partial class CookieJsonContext : JsonSerializerContext;
