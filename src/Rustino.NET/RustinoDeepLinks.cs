using System.Text;
using System.Text.Json;

namespace Rustino.NET;

/// <summary>Filters custom-scheme URLs from process arguments.</summary>
/// <remarks>
/// This helper does not register the scheme with the operating system. The app or its installer must
/// register the scheme separately (or declare it in the macOS app bundle).
/// </remarks>
public static class RustinoDeepLinks
{
    private const int MaximumUrlLength = 32 * 1024;
    private const int MaximumUrlCount = 128;
    private const int MaximumBatchBytes = 1024 * 1024;

    /// <summary>Returns process arguments that are absolute URLs using one of the supplied schemes.</summary>
    public static IReadOnlyList<string> GetCurrent(params string[] allowedSchemes)
    {
        var schemes = new HashSet<string>(NormalizeSchemes(allowedSchemes), StringComparer.OrdinalIgnoreCase);
        return Filter(Environment.GetCommandLineArgs().Skip(1), schemes);
    }

    internal static string[] NormalizeSchemes(string[] allowedSchemes)
    {
        ArgumentNullException.ThrowIfNull(allowedSchemes);
        if (allowedSchemes.Length == 0)
            throw new ArgumentException("Provide at least one URL scheme.", nameof(allowedSchemes));

        var normalized = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var scheme in allowedSchemes)
        {
            if (scheme is null || !IsValidScheme(scheme))
                throw new ArgumentException($"'{scheme}' is not a valid URL scheme.", nameof(allowedSchemes));
            normalized.Add(scheme.ToLowerInvariant());
        }
        return normalized.ToArray();
    }

    internal static string[] Filter(IEnumerable<string> args, HashSet<string> schemes)
    {
        var urls = new List<string>();
        var batchBytes = 2; // JSON array brackets
        foreach (var value in args)
        {
            if (value is null || value.Length == 0 || Encoding.UTF8.GetByteCount(value) > MaximumUrlLength
                || value.Any(character => char.IsControl(character) || char.IsWhiteSpace(character)))
                continue;
            if (!Uri.TryCreate(value, UriKind.Absolute, out var uri) || !schemes.Contains(uri.Scheme))
                continue;

            var jsonValueBytes = JsonSerializer.SerializeToUtf8Bytes(value).Length;
            var addedBytes = jsonValueBytes + (urls.Count == 0 ? 0 : 1);
            if (batchBytes + addedBytes > MaximumBatchBytes)
                break;
            urls.Add(value);
            batchBytes += addedBytes;
            if (urls.Count == MaximumUrlCount)
                break;
        }
        return urls.ToArray();
    }

    private static bool IsValidScheme(string value)
    {
        if (value.Length == 0 || !IsAsciiLetter(value[0])) return false;
        for (var i = 1; i < value.Length; i++)
        {
            var c = value[i];
            if (!IsAsciiLetter(c) && (c < '0' || c > '9') && c is not '+' and not '-' and not '.')
                return false;
        }
        return true;
    }

    private static bool IsAsciiLetter(char value) => value is >= 'A' and <= 'Z' or >= 'a' and <= 'z';
}
