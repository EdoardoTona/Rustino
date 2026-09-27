namespace Rustino.NET;

/// <summary>The native Rustino window failed to start or run.</summary>
public sealed class RustinoException(string message) : Exception(message)
{
}
