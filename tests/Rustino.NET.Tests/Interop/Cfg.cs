using System.Runtime.InteropServices;

namespace Rustino.NET.Tests.Interop;

/// <summary>A compilation target, for evaluating <c>#[cfg(...)]</c> predicates.</summary>
internal sealed record RustTarget(string Os, string Arch)
{
    public string Family => Os == "windows" ? "windows" : "unix";
    public string Vendor => Os == "macos" ? "apple" : Os == "windows" ? "pc" : "unknown";
    public string Env => Os switch { "windows" => "msvc", "linux" => "gnu", _ => "" };
    public string Rid => $"{Os switch { "windows" => "win", "macos" => "osx", _ => "linux" }}-{(Arch == "x86_64" ? "x64" : "arm64")}";

    /// <summary>The targets Rustino ships native libraries for.</summary>
    public static readonly IReadOnlyList<RustTarget> Supported =
    [
        new("windows", "x86_64"), new("windows", "aarch64"),
        new("macos", "x86_64"), new("macos", "aarch64"),
        new("linux", "x86_64"), new("linux", "aarch64"),
    ];

    public static RustTarget Current => new(
        OperatingSystem.IsWindows() ? "windows" : OperatingSystem.IsMacOS() ? "macos" : "linux",
        RuntimeInformation.ProcessArchitecture == Architecture.Arm64 ? "aarch64" : "x86_64");
}

/// <summary>
/// Evaluates <c>cfg</c> predicates (<c>all</c>, <c>any</c>, <c>not</c>, <c>target_os = ".."</c>, ...)
/// for a release build of a target with the given cargo features. Unknown predicates throw, so
/// a new kind of gating is noticed instead of guessed.
/// </summary>
internal static class Cfg
{
    public static bool Evaluate(string predicate, RustTarget target, IReadOnlySet<string> features)
    {
        var pos = 0;
        var result = Parse(predicate, ref pos, target, features);
        SkipWs(predicate, ref pos);
        if (pos != predicate.Length)
            throw new FormatException($"unexpected text in cfg({predicate}) at {pos}");
        return result;
    }

    public static bool EvaluateAll(IEnumerable<string> predicates, RustTarget target, IReadOnlySet<string> features) =>
        predicates.All(p => Evaluate(p, target, features));

    private static void SkipWs(string s, ref int pos)
    {
        while (pos < s.Length && char.IsWhiteSpace(s[pos])) pos++;
    }

    private static string Ident(string s, ref int pos)
    {
        SkipWs(s, ref pos);
        var start = pos;
        while (pos < s.Length && (char.IsLetterOrDigit(s[pos]) || s[pos] == '_')) pos++;
        if (start == pos) throw new FormatException($"expected an identifier in cfg({s}) at {pos}");
        return s[start..pos];
    }

    private static bool Parse(string s, ref int pos, RustTarget target, IReadOnlySet<string> features)
    {
        var name = Ident(s, ref pos);
        SkipWs(s, ref pos);
        if (pos < s.Length && s[pos] == '(')
        {
            pos++;
            var values = new List<bool>();
            while (true)
            {
                SkipWs(s, ref pos);
                if (pos < s.Length && s[pos] == ')') { pos++; break; }
                values.Add(Parse(s, ref pos, target, features));
                SkipWs(s, ref pos);
                if (pos < s.Length && s[pos] == ',') pos++;
            }
            return name switch
            {
                "all" => values.All(v => v),
                "any" => values.Any(v => v),
                "not" when values.Count == 1 => !values[0],
                _ => throw new NotSupportedException($"unsupported cfg function `{name}` in cfg({s})"),
            };
        }
        if (pos < s.Length && s[pos] == '=')
        {
            pos++;
            SkipWs(s, ref pos);
            if (pos >= s.Length || s[pos] != '"') throw new FormatException($"expected a string in cfg({s})");
            var end = s.IndexOf('"', pos + 1);
            var value = s[(pos + 1)..end];
            pos = end + 1;
            return name switch
            {
                "target_os" => value == target.Os,
                "target_family" => value == target.Family,
                "target_arch" => value == target.Arch,
                "target_vendor" => value == target.Vendor,
                "target_env" => value == target.Env,
                "target_pointer_width" => value == "64",
                "feature" => features.Contains(value),
                _ => throw new NotSupportedException($"unsupported cfg key `{name}` in cfg({s}): extend Cfg.Evaluate"),
            };
        }
        return name switch
        {
            "windows" => target.Family == "windows",
            "unix" => target.Family == "unix",
            // Exports are checked as they are in a release build of the library
            "test" or "debug_assertions" or "doc" or "miri" => false,
            _ => throw new NotSupportedException($"unsupported cfg `{name}` in cfg({s}): extend Cfg.Evaluate"),
        };
    }
}
