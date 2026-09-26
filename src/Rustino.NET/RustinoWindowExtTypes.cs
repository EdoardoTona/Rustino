namespace Rustino.NET;

public enum WindowTheme
{
    /// <summary>Follows the system theme (only as an argument of <c>SetTheme</c>).</summary>
    System = 0,
    Light = 1,
    Dark = 2,
}

public enum ProgressBarState
{
    None = 0,
    Normal = 1,
    /// <summary>Shown as <see cref="Normal"/> on Linux and macOS.</summary>
    Indeterminate = 2,
    /// <summary>Shown as <see cref="Normal"/> on Linux.</summary>
    Paused = 3,
    /// <summary>Shown as <see cref="Normal"/> on Linux.</summary>
    Error = 4,
}

public enum UserAttentionType
{
    /// <summary>Flashes the taskbar button once on Windows, bounces the Dock icon once on macOS.</summary>
    Informational = 1,
    /// <summary>Flashes the window and the taskbar button, or bounces the Dock icon, until the app is focused.</summary>
    Critical = 2,
}

public enum ResizeDirection
{
    North = 0,
    NorthEast = 1,
    East = 2,
    SouthEast = 3,
    South = 4,
    SouthWest = 5,
    West = 6,
    NorthWest = 7,
}

public enum MacTitleBarStyle
{
    Default = 0,
    /// <summary>Transparent title bar over the window background, with the title.</summary>
    Transparent = 1,
    /// <summary>
    /// The page extends under a transparent title bar without title, where only the traffic lights remain
    /// (the look of Slack or VS Code). Mark the page's title bar with <c>data-rustino-drag-region</c>.
    /// </summary>
    Overlay = 2,
}

public class ScaleFactorChangedEventArgs(double scaleFactor, int width, int height) : EventArgs
{
    public double ScaleFactor { get; } = scaleFactor;

    /// <summary>New size of the window content, in physical pixels.</summary>
    public int Width { get; } = width;

    public int Height { get; } = height;
}
