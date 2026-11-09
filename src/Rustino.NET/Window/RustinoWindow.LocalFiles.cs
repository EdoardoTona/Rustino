namespace Rustino.NET;

// Local files are served through the rustino-file scheme instead of file:// URLs. The webviews don't
// deliver window.ipc messages from file:// pages (wry builds an HTTP request from the page URL: macOS
// drops the message, Linux and Windows abort the process), and a scheme gives the pages a real origin.
// Only the folders of the files passed to Load are served, after resolving links.
public partial class RustinoWindow
{
    internal const string LocalFileScheme = "rustino-file";

    private static readonly StringComparison PathComparison =
        OperatingSystem.IsWindows() || OperatingSystem.IsMacOS() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal;

    private readonly HashSet<string> _localFileRoots = new(OperatingSystem.IsLinux() ? StringComparer.Ordinal : StringComparer.OrdinalIgnoreCase);

    // The URL serving a local file; its folder can then be served
    private string LocalFileUrl(string fullPath, string queryAndFragment = "")
    {
        lock (_localFileRoots)
            _localFileRoots.Add(Path.GetDirectoryName(fullPath) ?? fullPath);
        return LocalFileUrlOf(fullPath) + queryAndFragment;
    }

    // Each segment escaped, '%' included: a file named literal%20name.html keeps its name
    internal static string LocalFileUrlOf(string fullPath)
    {
        var segments = fullPath.Split(OperatingSystem.IsWindows() ? ['\\', '/'] : ['/'], StringSplitOptions.RemoveEmptyEntries);
        var path = string.Join('/', segments.Select((segment, i) =>
            // The drive of C:/app/index.html stays as it is
            i == 0 && OperatingSystem.IsWindows() && segment.EndsWith(':') ? segment : Uri.EscapeDataString(segment)));
        return $"{LocalFileScheme}://localhost/{path}";
    }

    // Null (404) outside the served folders
    internal static string? LocalFilePathOf(string url, IEnumerable<string> roots)
    {
        if (!Uri.TryCreate(url, UriKind.Absolute, out var uri)
            || !uri.Scheme.Equals(LocalFileScheme, StringComparison.OrdinalIgnoreCase))
            return null;
        var path = Uri.UnescapeDataString(uri.AbsolutePath);
        if (path.Contains('\0')) return null;
        if (OperatingSystem.IsWindows())
            path = path.TrimStart('/').Replace('/', '\\');
        if (!Path.IsPathRooted(path)) return null;
        path = Path.GetFullPath(path);
        // Where the file really is: a link in a served folder may lead anywhere
        if (RealPath(path) is not { } realPath) return null;
        foreach (var root in roots)
        {
            if (RealPath(root) is not { } realRoot) continue;
            var folder = Path.EndsInDirectorySeparator(realRoot) ? realRoot : realRoot + Path.DirectorySeparatorChar;
            if (realPath.StartsWith(folder, PathComparison))
                return path;
        }
        return null;
    }

    // The path with every link resolved (symbolic links, junctions), like realpath; null for link loops
    private static string? RealPath(string fullPath, int depth = 0)
    {
        if (depth > 40) return null;
        var root = Path.GetPathRoot(fullPath) ?? "";
        var current = root;
        foreach (var part in fullPath[root.Length..].Split(Path.DirectorySeparatorChar, StringSplitOptions.RemoveEmptyEntries))
        {
            current = Path.Combine(current, part);
            FileSystemInfo info = Directory.Exists(current) ? new DirectoryInfo(current) : new FileInfo(current);
            if (info.LinkTarget is null) continue;
            try
            {
                if (info.ResolveLinkTarget(returnFinalTarget: true) is not { } target) return null;
                // The target's own folders may be links too
                if (RealPath(target.FullName, depth + 1) is not { } resolved) return null;
                current = resolved;
            }
            catch (IOException) { return null; }
        }
        return current;
    }

    private Stream? ServeLocalFile(object sender, string scheme, string url, out string? contentType)
    {
        contentType = null;
        string[] roots;
        lock (_localFileRoots)
            roots = [.. _localFileRoots];
        var path = LocalFilePathOf(url, roots);
        if (path is null || !File.Exists(path)) return null;
        contentType = ContentTypeOf(path);
        return File.OpenRead(path);
    }

    private static string ContentTypeOf(string path) => Path.GetExtension(path).ToLowerInvariant() switch
    {
        ".html" or ".htm" => "text/html; charset=utf-8",
        ".js" or ".mjs" => "text/javascript; charset=utf-8",
        ".css" => "text/css; charset=utf-8",
        ".json" or ".map" => "application/json; charset=utf-8",
        ".txt" => "text/plain; charset=utf-8",
        ".xml" => "application/xml",
        ".svg" => "image/svg+xml",
        ".png" => "image/png",
        ".jpg" or ".jpeg" => "image/jpeg",
        ".gif" => "image/gif",
        ".webp" => "image/webp",
        ".avif" => "image/avif",
        ".ico" => "image/x-icon",
        ".bmp" => "image/bmp",
        ".wasm" => "application/wasm",
        ".woff" => "font/woff",
        ".woff2" => "font/woff2",
        ".ttf" => "font/ttf",
        ".otf" => "font/otf",
        ".mp4" => "video/mp4",
        ".webm" => "video/webm",
        ".mp3" => "audio/mpeg",
        ".wav" => "audio/wav",
        ".ogg" => "audio/ogg",
        ".pdf" => "application/pdf",
        _ => "application/octet-stream",
    };
}
