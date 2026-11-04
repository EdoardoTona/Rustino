using System.Text;
using System.Text.Json.Serialization;

namespace Rustino.NET;

public class SizeEventArgs(int width, int height) : EventArgs
{
    public int Width { get; } = width;
    public int Height { get; } = height;
}

public class PointEventArgs(int x, int y) : EventArgs
{
    public int X { get; } = x;
    public int Y { get; } = y;
}

public class NavigationEventArgs(string url) : System.ComponentModel.CancelEventArgs
{
    public string Url { get; } = url;
}

public class WebMessageEventArgs(string message, string sourceUrl) : EventArgs
{
    public string Message { get; } = message;
    // URL of the page that sent the message (custom schemes as `scheme://...` on every platform)
    public string SourceUrl { get; } = sourceUrl;
}

public class PageLoadEventArgs(bool isStarted, string url) : EventArgs
{
    public bool IsStarted { get; } = isStarted;
    public bool IsFinished => !IsStarted;
    public string Url { get; } = url;
}

public class MenuItemCheckedEventArgs(string id, bool isChecked) : EventArgs
{
    public string Id { get; } = id;
    public bool IsChecked { get; } = isChecked;
}

public enum TrayMouseButton
{
    Left,
    Right,
    Middle,
}

public class TrayIconClickedEventArgs(TrayMouseButton button, int x, int y) : EventArgs
{
    public TrayMouseButton Button { get; } = button;

    // Cursor position in physical pixels
    public int X { get; } = x;
    public int Y { get; } = y;
}

public class MonitorInfo
{
    [JsonPropertyName("name")]
    public string? Name { get; set; }

    [JsonPropertyName("x")]
    public int X { get; set; }

    [JsonPropertyName("y")]
    public int Y { get; set; }

    [JsonPropertyName("width")]
    public int Width { get; set; }

    [JsonPropertyName("height")]
    public int Height { get; set; }

    [JsonPropertyName("scaleFactor")]
    public double ScaleFactor { get; set; }

    [JsonPropertyName("isPrimary")]
    public bool IsPrimary { get; set; }

    public override string ToString() =>
        $"{Name ?? "Unknown"} ({Width}x{Height} at {X},{Y}, {ScaleFactor:F2}x{(IsPrimary ? ", primary" : "")})";
}

/// <summary>
/// The monitors after a change, and what changed. Monitors are matched by name (those with the same name, e.g. of
/// the same model, also by bounds): a monitor that kept its name but changed its bounds, scale factor or primary
/// status is in <see cref="Changed"/>.
/// </summary>
public class MonitorsChangedEventArgs(MonitorInfo[] monitors, MonitorInfo[] added, MonitorInfo[] removed, MonitorInfo[] changed) : EventArgs
{
    public MonitorInfo[] Monitors { get; } = monitors;
    public MonitorInfo[] Added { get; } = added;
    public MonitorInfo[] Removed { get; } = removed;
    public MonitorInfo[] Changed { get; } = changed;

    // Null when nothing changed. Identical monitors are paired first, then the others by name, one to one, so that
    // monitors of the same model (same name) are told apart by their bounds.
    internal static MonitorsChangedEventArgs? Compare(MonitorInfo[] previous, MonitorInfo[] current)
    {
        var unpaired = previous.ToList();
        var rest = new List<MonitorInfo>();
        foreach (var monitor in current)
        {
            var index = unpaired.FindIndex(p => p.Name == monitor.Name && SameMetrics(p, monitor));
            if (index >= 0) unpaired.RemoveAt(index);
            else rest.Add(monitor);
        }
        var added = new List<MonitorInfo>();
        var changed = new List<MonitorInfo>();
        foreach (var monitor in rest)
        {
            var index = unpaired.FindIndex(p => p.Name == monitor.Name);
            if (index >= 0)
            {
                unpaired.RemoveAt(index);
                changed.Add(monitor);
            }
            else added.Add(monitor);
        }
        if (added.Count == 0 && changed.Count == 0 && unpaired.Count == 0)
            return null;
        return new MonitorsChangedEventArgs(current, [.. added], [.. unpaired], [.. changed]);
    }

    private static bool SameMetrics(MonitorInfo a, MonitorInfo b) =>
        a.X == b.X && a.Y == b.Y && a.Width == b.Width && a.Height == b.Height
        && a.ScaleFactor == b.ScaleFactor && a.IsPrimary == b.IsPrimary;
}

[JsonSerializable(typeof(MonitorInfo))]
[JsonSerializable(typeof(MonitorInfo[]))]
internal partial class MonitorJsonContext : JsonSerializerContext;

public class FileFilter(string name, params string[] extensions)
{
    public string Name { get; } = name;
    public string[] Extensions { get; } = extensions;

    internal static string? Encode(FileFilter[]? filters)
    {
        if (filters is not { Length: > 0 }) return null;
        var sb = new StringBuilder();
        for (var i = 0; i < filters.Length; i++)
        {
            if (i > 0) sb.Append(';');
            sb.Append(filters[i].Name);
            sb.Append('|');
            sb.Append(string.Join(',', filters[i].Extensions));
        }
        return sb.ToString();
    }
}
