using System.Text;

namespace Rustino.NET.Tests.Interop;

/// <summary>A Rust type as written in a signature (only the shapes that can cross an FFI boundary).</summary>
internal abstract record RustType
{
    public abstract override string ToString();
}

internal sealed record RustUnit : RustType
{
    public override string ToString() => "()";
}

internal sealed record RustNever : RustType
{
    public override string ToString() => "!";
}

/// <summary><c>*const T</c> / <c>*mut T</c>; references and <c>NonNull</c> resolve to pointers too.</summary>
internal sealed record RustPointer(bool Mutable, RustType Pointee) : RustType
{
    public override string ToString() => (Mutable ? "*mut " : "*const ") + Pointee;
}

internal sealed record RustReference(bool Mutable, RustType Referent) : RustType
{
    public override string ToString() => (Mutable ? "&mut " : "&") + Referent;
}

/// <summary>A named type; <see cref="Name"/> is the last path segment (<c>window::SchemeResponse</c> → SchemeResponse).</summary>
internal sealed record RustPath(string Name, IReadOnlyList<RustType> Args) : RustType
{
    public override string ToString() =>
        Args.Count == 0 ? Name : $"{Name}<{string.Join(", ", Args)}>";

    public bool Equals(RustPath? other) =>
        other is not null && Name == other.Name && Args.SequenceEqual(other.Args);

    public override int GetHashCode() => Name.GetHashCode();
}

/// <summary>
/// A function pointer. <see cref="Abi"/> is null for the Rust ABI; <see cref="Nullable"/> when
/// wrapped in <c>Option</c>; <see cref="Alias"/> is the type alias it was declared through, if any.
/// </summary>
internal sealed record RustFunction(string? Abi, IReadOnlyList<RustType> Params, RustType Return) : RustType
{
    public bool Nullable { get; init; }
    public string? Alias { get; init; }

    public override string ToString()
    {
        var sb = new StringBuilder();
        if (Nullable) sb.Append("Option<");
        sb.Append(Abi is null ? "fn(" : $"extern \"{Abi}\" fn(");
        sb.Append(string.Join(", ", Params)).Append(')');
        if (Return is not RustUnit) sb.Append(" -> ").Append(Return);
        if (Nullable) sb.Append('>');
        return sb.ToString();
    }

    public bool Equals(RustFunction? other) =>
        other is not null && Abi == other.Abi && Params.SequenceEqual(other.Params) && Return.Equals(other.Return)
        && Nullable == other.Nullable;

    public override int GetHashCode() => Params.Count;
}

/// <summary>Anything else (tuples, arrays, trait objects): never valid across the boundary.</summary>
internal sealed record RustUnsupported(string Text) : RustType
{
    public override string ToString() => Text;
}

/// <summary>Recursive-descent parser for type text taken from comment-free source.</summary>
internal sealed class RustTypeParser
{
    private readonly List<string> _tokens;
    private int _pos;
    private readonly string _text;

    private RustTypeParser(string text)
    {
        _text = text;
        _tokens = Tokenize(text);
    }

    public static RustType Parse(string text)
    {
        var parser = new RustTypeParser(text);
        var type = parser.ParseType();
        if (parser._pos != parser._tokens.Count)
            throw new FormatException($"unexpected '{parser._tokens[parser._pos]}' in type `{text}`");
        return type;
    }

    private static List<string> Tokenize(string text)
    {
        var tokens = new List<string>();
        var i = 0;
        while (i < text.Length)
        {
            var c = text[i];
            if (char.IsWhiteSpace(c)) { i++; continue; }
            if (char.IsLetterOrDigit(c) || c == '_')
            {
                var j = i;
                while (j < text.Length && (char.IsLetterOrDigit(text[j]) || text[j] == '_')) j++;
                tokens.Add(text[i..j]);
                i = j;
            }
            else if (c == '"')
            {
                var j = text.IndexOf('"', i + 1);
                if (j < 0) throw new FormatException($"unterminated string in type `{text}`");
                tokens.Add(text[i..(j + 1)]);
                i = j + 1;
            }
            else if (c == '\'')
            {
                // Lifetime
                var j = i + 1;
                while (j < text.Length && (char.IsLetterOrDigit(text[j]) || text[j] == '_')) j++;
                tokens.Add(text[i..j]);
                i = j;
            }
            else if (text.AsSpan(i).StartsWith("::") || text.AsSpan(i).StartsWith("->"))
            {
                tokens.Add(text.Substring(i, 2));
                i += 2;
            }
            else if (text.AsSpan(i).StartsWith("..."))
            {
                tokens.Add("...");
                i += 3;
            }
            else
            {
                tokens.Add(c.ToString());
                i++;
            }
        }
        return tokens;
    }

    private string? Peek(int ahead = 0) => _pos + ahead < _tokens.Count ? _tokens[_pos + ahead] : null;

    private string Next() =>
        _pos < _tokens.Count ? _tokens[_pos++] : throw new FormatException($"unexpected end of type `{_text}`");

    private void Expect(string token)
    {
        var t = Next();
        if (t != token) throw new FormatException($"expected '{token}' but found '{t}' in type `{_text}`");
    }

    private RustType ParseType()
    {
        var t = Peek() ?? throw new FormatException($"missing type in `{_text}`");
        switch (t)
        {
            case "*":
            {
                Next();
                var qualifier = Next();
                if (qualifier is not ("const" or "mut"))
                    throw new FormatException($"expected const or mut after '*' in `{_text}`");
                return new RustPointer(qualifier == "mut", ParseType());
            }
            case "&":
            {
                Next();
                if (Peek()?.StartsWith('\'') == true) Next();
                var mutable = Peek() == "mut";
                if (mutable) Next();
                return new RustReference(mutable, ParseType());
            }
            case "(":
            {
                Next();
                if (Peek() == ")") { Next(); return new RustUnit(); }
                var items = new List<RustType> { ParseType() };
                var trailingComma = false;
                while (Peek() == ",")
                {
                    Next();
                    trailingComma = true;
                    if (Peek() == ")") break;
                    items.Add(ParseType());
                    trailingComma = false;
                }
                Expect(")");
                return items.Count == 1 && !trailingComma
                    ? items[0]
                    : new RustUnsupported($"({string.Join(", ", items)})");
            }
            case "!":
                Next();
                return new RustNever();
            case "[":
            {
                var start = _pos;
                SkipBalanced("[", "]");
                return new RustUnsupported(string.Join("", _tokens.GetRange(start, _pos - start)));
            }
            case "unsafe" or "extern" or "fn" or "for":
                return ParseFunction();
            case "dyn" or "impl":
            {
                var start = _pos;
                while (Peek() is { } p && p is not ("," or ")" or ">")) Next();
                return new RustUnsupported(string.Join(" ", _tokens.GetRange(start, _pos - start)));
            }
            default:
                return ParsePath();
        }
    }

    private void SkipBalanced(string open, string close)
    {
        var depth = 0;
        do
        {
            var t = Next();
            if (t == open) depth++;
            else if (t == close) depth--;
        } while (depth > 0);
    }

    private RustType ParseFunction()
    {
        if (Peek() == "for")
        {
            // Higher-ranked lifetimes: for<'a>
            Next();
            SkipBalanced("<", ">");
        }
        if (Peek() == "unsafe") Next();
        string? abi = null;
        if (Peek() == "extern")
        {
            Next();
            abi = "C";
            if (Peek()?.StartsWith('"') == true) abi = Next().Trim('"');
        }
        Expect("fn");
        Expect("(");
        var parameters = new List<RustType>();
        while (Peek() != ")")
        {
            if (Peek() == "...")
            {
                Next();
                parameters.Add(new RustUnsupported("..."));
            }
            else
            {
                // Optional parameter name: `context: *const c_void`
                if (Peek(1) == ":" && Peek() is { } name && (char.IsLetter(name[0]) || name[0] == '_'))
                {
                    Next();
                    Next();
                }
                parameters.Add(ParseType());
            }
            if (Peek() == ",") Next();
            else break;
        }
        Expect(")");
        RustType ret = new RustUnit();
        if (Peek() == "->")
        {
            Next();
            ret = ParseType();
        }
        return new RustFunction(abi, parameters, ret);
    }

    private RustType ParsePath()
    {
        if (Peek() == "::") Next();
        var name = Next();
        if (!(char.IsLetter(name[0]) || name[0] == '_'))
            throw new FormatException($"unexpected '{name}' in type `{_text}`");
        while (Peek() == "::" && Peek(1) != "<")
        {
            Next();
            name = Next();
        }
        if (Peek() == "::") Next(); // turbofish-style `Option::<T>`
        var args = new List<RustType>();
        if (Peek() == "<")
        {
            Next();
            while (Peek() != ">")
            {
                if (Peek()?.StartsWith('\'') == true) Next(); // lifetime argument
                else args.Add(ParseType());
                if (Peek() == ",") Next();
                else break;
            }
            Expect(">");
        }
        return new RustPath(name, args);
    }
}
