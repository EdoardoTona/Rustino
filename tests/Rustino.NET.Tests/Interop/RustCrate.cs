using System.Text.RegularExpressions;

namespace Rustino.NET.Tests.Interop;

internal sealed record RustParam(string Name, RustType Type, string Text);

/// <summary>A <c>#[no_mangle]</c> function. <see cref="Cfgs"/> holds every cfg predicate gating it (its own and its modules').</summary>
internal sealed record RustExport(
    string Name,
    string? Abi,
    IReadOnlyList<RustParam> Params,
    RustType Return,
    IReadOnlyList<string> Cfgs,
    string File,
    int Line)
{
    public string Location => $"{File}:{Line}";

    public bool IsEnabledOn(RustTarget target, IReadOnlySet<string> features) =>
        Cfg.EvaluateAll(Cfgs, target, features);

    public string Signature =>
        $"{(Abi is null ? "" : $"extern \"{Abi}\" ")}fn {Name}({string.Join(", ", Params.Select(p => $"{p.Name}: {p.Type}"))})"
        + (Return is RustUnit ? "" : $" -> {Return}");
}

internal sealed record RustField(string Name, RustType Type, string Text);

internal sealed record RustStruct(string Name, string Repr, IReadOnlyList<RustField> Fields, IReadOnlyList<string> Cfgs, string File, int Line)
{
    public string Location => $"{File}:{Line}";
}

internal sealed record RustAlias(string Name, string Text, string File, int Line);

/// <summary>
/// The FFI surface of a Rust crate, read from its sources: the files reachable from
/// <c>src/lib.rs</c> through <c>mod</c> declarations, with the cfg gating of each module.
/// Items are recognized structurally (no macro expansion), and anything the parser could not
/// account for is reported in <see cref="Problems"/> rather than silently skipped.
/// </summary>
internal sealed class RustCrate
{
    private static readonly Regex NoMangle = new(@"#\s*\[\s*(unsafe\s*\(\s*)?no_mangle\s*\)?\s*\]", RegexOptions.Compiled);
    private static readonly Regex ReprC = new(@"#\s*\[\s*repr\s*\(\s*C\b", RegexOptions.Compiled);

    public string CrateDir { get; }
    public List<RustExport> Exports { get; } = [];
    public List<RustStruct> Structs { get; } = [];
    public Dictionary<string, List<RustAlias>> Aliases { get; } = [];
    public HashSet<string> DefaultFeatures { get; } = [];
    public HashSet<string> AllFeatures { get; } = [];
    public List<string> Problems { get; } = [];
    public List<string> Files { get; } = [];

    private RustCrate(string crateDir) => CrateDir = crateDir;

    private sealed record ModuleContext(string File, IReadOnlyList<string> InlinePath, List<string> Cfgs);

    private sealed record ChildModule(string Name, string? PathAttr, ModuleContext Parent, List<string> Cfgs);

    public static RustCrate Load(string crateDir)
    {
        var crate = new RustCrate(crateDir);
        crate.ReadDefaultFeatures();
        var root = Path.Combine(crateDir, "src", "lib.rs");
        if (!File.Exists(root))
        {
            crate.Problems.Add($"{root} not found");
            return crate;
        }

        var queue = new Queue<(string Path, List<string> Cfgs)>();
        queue.Enqueue((root, []));
        var seen = new HashSet<string>(StringComparer.Ordinal);
        while (queue.Count > 0)
        {
            var (path, cfgs) = queue.Dequeue();
            if (!seen.Add(Path.GetFullPath(path))) continue;
            foreach (var child in crate.ParseFile(path, cfgs))
            {
                var resolved = crate.ResolveModuleFile(child);
                if (resolved is null)
                {
                    crate.Problems.Add(
                        $"{crate.Relative(child.Parent.File)}: file of `mod {child.Name};` not found");
                    continue;
                }
                queue.Enqueue((resolved, child.Cfgs));
            }
        }

        // An export in a file outside the module tree would silently escape the checks
        foreach (var file in Directory.EnumerateFiles(Path.Combine(crateDir, "src"), "*.rs", SearchOption.AllDirectories))
        {
            if (seen.Contains(Path.GetFullPath(file))) continue;
            var text = new RustText(File.ReadAllText(file));
            if (NoMangle.IsMatch(text.Skeleton))
                crate.Problems.Add($"{crate.Relative(file)} has #[no_mangle] items but is not reachable from src/lib.rs through `mod` declarations");
        }
        return crate;
    }

    /// <summary>Parses Rust source as the crate root (for the checker's own tests).</summary>
    public static RustCrate FromSource(string source)
    {
        var dir = Directory.CreateTempSubdirectory("rustino-contract-").FullName;
        Directory.CreateDirectory(Path.Combine(dir, "src"));
        File.WriteAllText(Path.Combine(dir, "src", "lib.rs"), source);
        try
        {
            return Load(dir);
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    public string Relative(string path) =>
        Path.GetRelativePath(CrateDir, path).Replace('\\', '/');

    private void ReadDefaultFeatures()
    {
        var manifest = Path.Combine(CrateDir, "Cargo.toml");
        if (!File.Exists(manifest)) return;
        var text = File.ReadAllText(manifest);
        var section = Regex.Match(text, @"^\[features\]\s*$(?<body>.*?)(?=^\[|\z)", RegexOptions.Multiline | RegexOptions.Singleline);
        if (!section.Success) return;
        foreach (Match m in Regex.Matches(section.Groups["body"].Value, @"^\s*(?<name>[A-Za-z0-9_-]+)\s*=", RegexOptions.Multiline))
            if (m.Groups["name"].Value != "default")
                AllFeatures.Add(m.Groups["name"].Value);
        var defaults = Regex.Match(section.Groups["body"].Value, @"^\s*default\s*=\s*\[(?<list>[^\]]*)\]", RegexOptions.Multiline);
        if (!defaults.Success) return;
        foreach (Match m in Regex.Matches(defaults.Groups["list"].Value, "\"([^\"]+)\""))
            DefaultFeatures.Add(m.Groups[1].Value);
    }

    private string? ResolveModuleFile(ChildModule child)
    {
        var parentFile = child.Parent.File;
        var parentDir = Path.GetDirectoryName(parentFile)!;
        if (child.PathAttr is not null)
            return File.Exists(Path.Combine(parentDir, child.PathAttr)) ? Path.Combine(parentDir, child.PathAttr) : null;

        // lib.rs, main.rs and mod.rs own their directory; foo.rs owns foo/
        var stem = Path.GetFileNameWithoutExtension(parentFile);
        var dir = stem is "lib" or "main" or "mod" ? parentDir : Path.Combine(parentDir, stem);
        foreach (var segment in child.Parent.InlinePath) dir = Path.Combine(dir, segment);
        var flat = Path.Combine(dir, child.Name + ".rs");
        if (File.Exists(flat)) return flat;
        var nested = Path.Combine(dir, child.Name, "mod.rs");
        return File.Exists(nested) ? nested : null;
    }

    private List<ChildModule> ParseFile(string path, List<string> cfgs)
    {
        Files.Add(Relative(path));
        var text = new RustText(File.ReadAllText(path));
        var children = new List<ChildModule>();
        var exportsBefore = Exports.Count;
        var structsBefore = Structs.Count;
        var unsupportedExports = 0;
        try
        {
            var ctx = new ModuleContext(path, [], [.. cfgs]);
            ParseItems(text, 0, text.Skeleton.Length, ctx, children, ref unsupportedExports);
        }
        catch (FormatException e)
        {
            Problems.Add($"{Relative(path)}: cannot parse: {e.Message}");
            return children;
        }

        // Safety net: every attribute found in the text must belong to an item that was parsed
        var noMangle = NoMangle.Matches(text.Skeleton).Count;
        var parsedExports = Exports.Count - exportsBefore + unsupportedExports;
        if (noMangle != parsedExports)
            Problems.Add($"{Relative(path)}: {noMangle} #[no_mangle] attributes but {parsedExports} exported functions parsed (an export inside an impl, fn body or macro is not supported)");
        var reprC = ReprC.Matches(text.Skeleton).Count;
        var parsedStructs = Structs.Count - structsBefore;
        if (reprC != parsedStructs)
            Problems.Add($"{Relative(path)}: {reprC} #[repr(C)] attributes but {parsedStructs} structs parsed (only named-field structs at module level are supported)");
        return children;
    }

    private static bool IsIdentStart(char c) => char.IsLetter(c) || c == '_';
    private static bool IsIdent(char c) => char.IsLetterOrDigit(c) || c == '_';

    private static int SkipWs(RustText t, int pos, int end)
    {
        while (pos < end && char.IsWhiteSpace(t.Skeleton[pos])) pos++;
        return pos;
    }

    private static (string Word, int End) ReadWord(RustText t, int pos, int end)
    {
        pos = SkipWs(t, pos, end);
        var start = pos;
        while (pos < end && IsIdent(t.Skeleton[pos])) pos++;
        return (t.Skeleton[start..pos], pos);
    }

    private static string? CfgOf(string attr)
    {
        var m = Regex.Match(attr, @"^cfg\s*\((?<p>.*)\)$", RegexOptions.Singleline);
        return m.Success ? m.Groups["p"].Value.Trim() : null;
    }

    private void ParseItems(RustText t, int start, int end, ModuleContext ctx, List<ChildModule> children, ref int unsupportedExports)
    {
        var attrs = new List<string>();
        var itemStart = -1;
        var pos = start;
        while (true)
        {
            pos = SkipWs(t, pos, end);
            if (pos >= end) break;
            var c = t.Skeleton[pos];

            if (c == '#')
            {
                var inner = pos + 1 < end && t.Skeleton[pos + 1] == '!';
                var open = SkipWs(t, pos + (inner ? 2 : 1), end);
                if (open >= end || t.Skeleton[open] != '[')
                    throw new FormatException($"expected '[' after '#' at line {t.LineOf(pos)}");
                var close = t.MatchClose(open);
                var attr = t.CodeBetween(open + 1, close);
                if (inner)
                {
                    if (CfgOf(attr) is { } cfg) ctx.Cfgs.Add(cfg);
                }
                else
                {
                    if (attrs.Count == 0) itemStart = pos;
                    attrs.Add(attr);
                }
                pos = close + 1;
                continue;
            }
            if (c == ';')
            {
                pos++;
                continue;
            }
            if (attrs.Count == 0) itemStart = pos;
            pos = ParseItem(t, pos, end, attrs, itemStart, ctx, children, ref unsupportedExports);
            attrs = [];
        }
    }

    // Parses the item at `pos` and returns the offset right after it
    private int ParseItem(RustText t, int pos, int end, List<string> attrs, int itemStart, ModuleContext ctx,
        List<ChildModule> children, ref int unsupportedExports)
    {
        var isExport = attrs.Any(a => Regex.IsMatch(a, @"^(unsafe\s*\(\s*)?no_mangle\s*\)?$"));
        var repr = attrs.FirstOrDefault(a => Regex.IsMatch(a, @"^repr\s*\(\s*C\b"));
        var cfgs = ctx.Cfgs.Concat(attrs.Select(CfgOf).OfType<string>()).ToList();
        string? kind = null;
        var p = pos;

        while (kind is null)
        {
            p = SkipWs(t, p, end);
            if (p >= end) throw new FormatException($"unexpected end of item at line {t.LineOf(pos)}");
            if (!IsIdentStart(t.Skeleton[p]))
            {
                kind = "other";
                break;
            }
            var (word, after) = ReadWord(t, p, end);
            switch (word)
            {
                case "pub":
                    p = SkipWs(t, after, end);
                    if (p < end && t.Skeleton[p] == '(') p = t.MatchClose(p) + 1;
                    break;
                case "unsafe" or "async" or "default" or "safe" or "auto":
                    p = after;
                    break;
                case "const":
                    var (nextWord, _) = ReadWord(t, after, end);
                    if (nextWord is "fn" or "unsafe" or "async" or "extern") p = after;
                    else kind = "const";
                    break;
                case "extern":
                    p = SkipWs(t, after, end);
                    if (p < end && t.Skeleton[p] == '"') p = t.Skeleton.IndexOf('"', p + 1) + 1;
                    p = SkipWs(t, p, end);
                    if (p < end && t.Skeleton[p] == '{') kind = "extern-block";
                    else if (ReadWord(t, p, end).Word == "crate") kind = "use";
                    break;
                case "fn" or "mod" or "struct" or "enum" or "union" or "trait" or "impl" or "type" or "use"
                    or "static" or "macro_rules":
                    kind = word;
                    p = after;
                    break;
                default:
                    kind = "other";
                    break;
            }
        }

        var semicolonOnly = kind is "const" or "static" or "use" or "type";
        var itemEnd = FindItemEnd(t, p, end, semicolonOnly, out var blockOpen);

        switch (kind)
        {
            case "fn" when isExport:
                Exports.Add(ParseExport(t, pos, blockOpen >= 0 ? blockOpen : itemEnd - 1, cfgs, ctx));
                return itemEnd;
            case "mod":
            {
                var (name, _) = ReadWord(t, p, end);
                if (blockOpen >= 0)
                {
                    var child = ctx with { InlinePath = [.. ctx.InlinePath, name], Cfgs = cfgs };
                    ParseItems(t, blockOpen + 1, itemEnd - 1, child, children, ref unsupportedExports);
                }
                else
                {
                    var pathAttr = attrs
                        .Select(a => Regex.Match(a, "^path\\s*=\\s*\"(?<p>[^\"]+)\"$"))
                        .FirstOrDefault(m => m.Success)?.Groups["p"].Value;
                    children.Add(new ChildModule(name, pathAttr, ctx, cfgs));
                }
                return itemEnd;
            }
            case "struct" when repr is not null:
                Structs.Add(ParseStruct(t, p, blockOpen, itemEnd, repr, cfgs, ctx, itemStart));
                return itemEnd;
            case "type":
            {
                var m = Regex.Match(t.CodeBetween(p, itemEnd - 1), @"^(?<name>\w+)\s*(<[^=]*>)?\s*=\s*(?<ty>.+)$", RegexOptions.Singleline);
                if (m.Success)
                {
                    var alias = new RustAlias(m.Groups["name"].Value, m.Groups["ty"].Value.Trim(), Relative(ctx.File), t.LineOf(pos));
                    if (!Aliases.TryGetValue(alias.Name, out var list)) Aliases[alias.Name] = list = [];
                    list.Add(alias);
                }
                return itemEnd;
            }
        }

        if (isExport)
        {
            unsupportedExports++;
            Problems.Add($"{Relative(ctx.File)}:{t.LineOf(pos)}: #[no_mangle] on a `{kind}` item is not supported by the interop checks (only functions are)");
        }
        return itemEnd;
    }

    // End of an item: the `;` at depth 0, or (unless semicolonOnly) the `}` closing its first top-level block
    private static int FindItemEnd(RustText t, int pos, int end, bool semicolonOnly, out int blockOpen)
    {
        blockOpen = -1;
        var k = pos;
        while (k < end)
        {
            var c = t.Skeleton[k];
            if (c is '(' or '[')
            {
                k = t.MatchClose(k) + 1;
                continue;
            }
            if (c == '{')
            {
                var close = t.MatchClose(k);
                if (!semicolonOnly)
                {
                    blockOpen = k;
                    return close + 1;
                }
                k = close + 1;
                continue;
            }
            if (c == ';') return k + 1;
            k++;
        }
        throw new FormatException($"item starting at line {t.LineOf(pos)} has no end");
    }

    private RustExport ParseExport(RustText t, int headerStart, int headerEnd, List<string> cfgs, ModuleContext ctx)
    {
        var header = t.Skeleton[headerStart..headerEnd];
        var fnMatch = Regex.Match(header, @"\bfn\s+(?<name>[A-Za-z_]\w*)");
        if (!fnMatch.Success) throw new FormatException($"no fn name at line {t.LineOf(headerStart)}");
        var name = fnMatch.Groups["name"].Value;
        var line = t.LineOf(headerStart);

        // ABI: `extern "C"` (strings are blank in the skeleton, read them from the code view)
        string? abi = null;
        var qualifiers = t.Code[headerStart..(headerStart + fnMatch.Index)];
        var abiMatch = Regex.Match(qualifiers, "\\bextern\\b\\s*(\"(?<abi>[^\"]*)\")?");
        if (abiMatch.Success) abi = abiMatch.Groups["abi"].Success ? abiMatch.Groups["abi"].Value : "C";

        var afterName = headerStart + fnMatch.Index + fnMatch.Length;
        var open = t.Skeleton.IndexOf('(', afterName);
        if (open < 0 || open > headerEnd) throw new FormatException($"no parameter list for {name} at line {line}");
        if (t.Skeleton[afterName..open].Contains('<'))
            throw new FormatException($"generic export {name} at line {line}");
        var close = t.MatchClose(open);

        var parameters = new List<RustParam>();
        foreach (var part in SplitTopLevel(t.CodeBetween(open + 1, close)))
        {
            var colon = FindSingleColon(part);
            if (colon < 0) throw new FormatException($"parameter `{part}` of {name} at line {line} has no type");
            var pname = part[..colon].Trim();
            if (pname.StartsWith("mut ")) pname = pname[4..].Trim();
            var ptype = part[(colon + 1)..].Trim();
            parameters.Add(new RustParam(pname, ParseTypeAt(ptype, name, line), ptype));
        }

        RustType ret = new RustUnit();
        var rest = t.CodeBetween(close + 1, headerEnd);
        var where = Regex.Match(rest, @"\bwhere\b");
        if (where.Success) rest = rest[..where.Index].Trim();
        if (rest.StartsWith("->")) ret = ParseTypeAt(rest[2..].Trim(), name, line);
        else if (rest.Length > 0) throw new FormatException($"unexpected `{rest}` after the parameters of {name} at line {line}");

        return new RustExport(name, abi, parameters, ret, cfgs, Relative(ctx.File), line);
    }

    private static RustType ParseTypeAt(string text, string owner, int line)
    {
        try
        {
            return RustTypeParser.Parse(text);
        }
        catch (FormatException e)
        {
            throw new FormatException($"{owner} at line {line}: {e.Message}");
        }
    }

    private RustStruct ParseStruct(RustText t, int afterKeyword, int blockOpen, int itemEnd, string repr,
        List<string> cfgs, ModuleContext ctx, int itemStart)
    {
        var (name, _) = ReadWord(t, afterKeyword, itemEnd);
        var line = t.LineOf(itemStart);
        if (blockOpen < 0)
            throw new FormatException($"#[repr(C)] struct {name} at line {line} has no named fields (tuple and unit structs are not supported)");
        var fields = new List<RustField>();
        foreach (var raw in SplitTopLevel(t.CodeBetween(blockOpen + 1, itemEnd - 1)))
        {
            // Drop field attributes and visibility
            var part = Regex.Replace(raw, @"#\s*\[[^\]]*\]", "").Trim();
            part = Regex.Replace(part, @"^pub(\s*\([^)]*\))?\s+", "");
            var colon = FindSingleColon(part);
            if (colon < 0) throw new FormatException($"field `{part}` of {name} at line {line} has no type");
            var ftext = part[(colon + 1)..].Trim();
            fields.Add(new RustField(part[..colon].Trim(), ParseTypeAt(ftext, name, line), ftext));
        }
        return new RustStruct(name, repr, fields, cfgs, Relative(ctx.File), line);
    }

    /// <summary>Splits on commas outside (), [], {} and generic &lt;&gt; (the '>' of '->' does not count).</summary>
    public static List<string> SplitTopLevel(string s)
    {
        var parts = new List<string>();
        var depth = 0;
        var start = 0;
        for (var i = 0; i < s.Length; i++)
        {
            var c = s[i];
            if (c is '(' or '[' or '{' or '<') depth++;
            else if (c == '>' && i > 0 && s[i - 1] == '-') { }
            else if (c is ')' or ']' or '}' or '>') depth--;
            else if (c == ',' && depth == 0)
            {
                parts.Add(s[start..i].Trim());
                start = i + 1;
            }
        }
        var last = s[start..].Trim();
        if (last.Length > 0) parts.Add(last);
        return parts.Where(x => x.Length > 0).ToList();
    }

    private static int FindSingleColon(string s)
    {
        for (var i = 0; i < s.Length; i++)
        {
            if (s[i] != ':') continue;
            if (i + 1 < s.Length && s[i + 1] == ':') { i++; continue; }
            return i;
        }
        return -1;
    }

    public RustStruct? FindStruct(string name) => Structs.FirstOrDefault(s => s.Name == name);

    /// <summary>
    /// Expands type aliases and the FFI-transparent wrappers: <c>Option&lt;fn&gt;</c> becomes a
    /// nullable function pointer, <c>&amp;T</c>/<c>NonNull&lt;T&gt;</c>/<c>Option&lt;&amp;T&gt;</c> become pointers.
    /// </summary>
    public RustType Resolve(RustType type, int depth = 0)
    {
        if (depth > 32) return new RustUnsupported($"{type} (alias cycle)");
        switch (type)
        {
            case RustPath { Args.Count: 0 } path when Aliases.TryGetValue(path.Name, out var aliases):
            {
                if (aliases.Select(a => a.Text).Distinct().Count() > 1)
                    return new RustUnsupported($"{path.Name} (ambiguous: declared as a type alias in {string.Join(", ", aliases.Select(a => $"{a.File}:{a.Line}"))})");
                RustType target;
                try
                {
                    target = RustTypeParser.Parse(aliases[0].Text);
                }
                catch (FormatException e)
                {
                    return new RustUnsupported($"{path.Name} ({e.Message})");
                }
                var resolved = Resolve(target, depth + 1);
                return resolved is RustFunction { Alias: null } fn ? fn with { Alias = path.Name } : resolved;
            }
            case RustPath { Name: "Option", Args.Count: 1 } option:
            {
                var inner = Resolve(option.Args[0], depth + 1);
                return inner switch
                {
                    RustFunction fn => fn with { Nullable = true },
                    RustPointer ptr => ptr,
                    _ => new RustUnsupported($"Option<{inner}> (not FFI-safe: only Option of fn pointers and references is)"),
                };
            }
            case RustPath { Name: "NonNull", Args.Count: 1 } nonNull:
                return new RustPointer(true, Resolve(nonNull.Args[0], depth + 1));
            case RustReference reference:
                return new RustPointer(reference.Mutable, Resolve(reference.Referent, depth + 1));
            case RustPointer pointer:
                return pointer with { Pointee = Resolve(pointer.Pointee, depth + 1) };
            case RustFunction fn:
                return fn with
                {
                    Params = fn.Params.Select(x => Resolve(x, depth + 1)).ToList(),
                    Return = Resolve(fn.Return, depth + 1),
                };
            default:
                return type;
        }
    }
}
