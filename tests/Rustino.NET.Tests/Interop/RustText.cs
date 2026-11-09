namespace Rustino.NET.Tests.Interop;

/// <summary>
/// A Rust source file in three aligned views of the same length, so offsets found in one view
/// index the others: <see cref="Original"/>; <see cref="Code"/> without comments; and
/// <see cref="Skeleton"/> without comments and without the contents of string and char literals,
/// where brackets can be matched blindly.
/// </summary>
internal sealed class RustText
{
    public string Original { get; }
    public string Code { get; }
    public string Skeleton { get; }

    public RustText(string original)
    {
        Original = original;
        var code = original.ToCharArray();
        var skeleton = original.ToCharArray();
        var n = original.Length;
        var i = 0;

        void Blank(char[] target, int from, int to)
        {
            for (var k = from; k < to && k < n; k++)
                if (target[k] != '\n' && target[k] != '\r')
                    target[k] = ' ';
        }

        while (i < n)
        {
            var c = original[i];
            var next = i + 1 < n ? original[i + 1] : '\0';

            if (c == '/' && next == '/')
            {
                var end = original.IndexOf('\n', i);
                if (end < 0) end = n;
                Blank(code, i, end);
                Blank(skeleton, i, end);
                i = end;
            }
            else if (c == '/' && next == '*')
            {
                // Block comments nest in Rust
                var depth = 1;
                var j = i + 2;
                while (j < n && depth > 0)
                {
                    if (original[j] == '/' && j + 1 < n && original[j + 1] == '*') { depth++; j += 2; }
                    else if (original[j] == '*' && j + 1 < n && original[j + 1] == '/') { depth--; j += 2; }
                    else j++;
                }
                Blank(code, i, j);
                Blank(skeleton, i, j);
                i = j;
            }
            else if (c == 'r' && IsRawStringStart(original, i))
            {
                var j = i + 1;
                var hashes = 0;
                while (j < n && original[j] == '#') { hashes++; j++; }
                var open = j; // the opening quote
                var terminator = "\"" + new string('#', hashes);
                var close = original.IndexOf(terminator, open + 1, StringComparison.Ordinal);
                if (close < 0) close = n;
                Blank(skeleton, open + 1, close);
                i = close + terminator.Length;
            }
            else if (c == '"')
            {
                var j = i + 1;
                while (j < n && original[j] != '"')
                    j += original[j] == '\\' ? 2 : 1;
                Blank(skeleton, i + 1, j);
                i = j + 1;
            }
            else if (c == '\'')
            {
                // Char literal ('x', '\n', '\u{..}', '"') or lifetime ('a)
                if (next == '\\')
                {
                    var close = original.IndexOf('\'', i + 3);
                    if (close < 0) close = n;
                    Blank(skeleton, i + 1, close);
                    i = close + 1;
                }
                else if (i + 2 < n && original[i + 2] == '\'')
                {
                    Blank(skeleton, i + 1, i + 2);
                    i += 3;
                }
                else if (i + 3 < n && char.IsHighSurrogate(next) && original[i + 3] == '\'')
                {
                    Blank(skeleton, i + 1, i + 3);
                    i += 4;
                }
                else
                {
                    i++;
                }
            }
            else
            {
                i++;
            }
        }

        Code = new string(code);
        Skeleton = new string(skeleton);
    }

    private static bool IsIdentChar(char c) => char.IsLetterOrDigit(c) || c == '_';

    // r"..", r#".."#, br"..": `r` must start a token (or follow a `b` that does)
    private static bool IsRawStringStart(string s, int i)
    {
        var start = i;
        if (i > 0 && s[i - 1] == 'b') start = i - 1;
        if (start > 0 && IsIdentChar(s[start - 1])) return false;
        var j = i + 1;
        while (j < s.Length && s[j] == '#') j++;
        return j < s.Length && s[j] == '"';
    }

    public int LineOf(int offset)
    {
        var line = 1;
        for (var k = 0; k < offset && k < Original.Length; k++)
            if (Original[k] == '\n') line++;
        return line;
    }

    /// <summary>Index of the bracket closing the one at <paramref name="open"/> (on the skeleton).</summary>
    public int MatchClose(int open)
    {
        var stack = new Stack<char>();
        for (var k = open; k < Skeleton.Length; k++)
        {
            var c = Skeleton[k];
            switch (c)
            {
                case '(': stack.Push(')'); break;
                case '[': stack.Push(']'); break;
                case '{': stack.Push('}'); break;
                case ')' or ']' or '}':
                    if (stack.Count == 0 || stack.Pop() != c)
                        throw new FormatException($"unbalanced '{c}' at line {LineOf(k)}");
                    if (stack.Count == 0) return k;
                    break;
            }
        }
        throw new FormatException($"unclosed '{Skeleton[open]}' opened at line {LineOf(open)}");
    }

    /// <summary>Comment-free source between two offsets, with whitespace collapsed.</summary>
    public string CodeBetween(int from, int to) => Collapse(Code[from..to]);

    public static string Collapse(string s) =>
        System.Text.RegularExpressions.Regex.Replace(s, @"\s+", " ").Trim();
}
