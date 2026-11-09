using System.Security.Cryptography;

namespace Rustino.NET.Tests;

/// <summary>
/// RustinoSafeStorage with a key of its own, deleted at the end. Without a key store (Linux without a Secret
/// Service) only the checks that need no key run.
/// </summary>
public sealed class SafeStorageTests : IDisposable
{
    private readonly RustinoSafeStorage _storage = new("it.ortarzo.rustino.safestoragetest");

    public void Dispose()
    {
        if (OperatingSystem.IsWindows() || _storage.IsEncryptionAvailable)
            _storage.ResetKey();
    }

    [Theory]
    [InlineData("")]
    [InlineData(".hidden")]
    [InlineData("a b")]
    [InlineData("a/b")]
    public void IdsThatCantNameAKeyAreRejected(string appId) =>
        Assert.Throws<ArgumentException>(() => new RustinoSafeStorage(appId));

    [Fact]
    public void WindowsAlwaysEncrypts()
    {
        if (OperatingSystem.IsWindows()) Assert.True(_storage.IsEncryptionAvailable);
    }

    [Fact]
    public void BytesThatArentEncryptedAreRejected()
    {
        Assert.Throws<CryptographicException>(() => _storage.Decrypt([]));
        Assert.Throws<CryptographicException>(() => _storage.Decrypt([42, 1, 2, 3]));
    }

    [Fact]
    public void TextRoundTrips()
    {
        if (!_storage.IsEncryptionAvailable) return;
        const string secret = "token: città 🌍";

        var encrypted = _storage.EncryptString(secret);

        Assert.True(encrypted.AsSpan().IndexOf("token"u8) < 0);
        Assert.Equal(secret, _storage.DecryptString(encrypted));
        // A fresh nonce (or DPAPI salt) each time
        Assert.NotEqual(encrypted, _storage.EncryptString(secret));
    }

    [Fact]
    public void EmptyDataRoundTrips()
    {
        if (!_storage.IsEncryptionAvailable) return;

        Assert.Empty(_storage.Decrypt(_storage.Encrypt([])));
    }

    [Fact]
    public void AlteredBytesAreRejected()
    {
        if (!_storage.IsEncryptionAvailable) return;
        var encrypted = _storage.Encrypt("secret"u8);

        encrypted[^1] ^= 1;

        Assert.Throws<CryptographicException>(() => _storage.Decrypt(encrypted));
    }

    [Fact]
    public void AnotherAppIdCantDecrypt()
    {
        if (!_storage.IsEncryptionAvailable) return;
        var other = new RustinoSafeStorage("it.ortarzo.rustino.safestoragetest2");
        try
        {
            var encrypted = _storage.Encrypt("secret"u8);
            Assert.Throws<CryptographicException>(() => other.Decrypt(encrypted));
        }
        finally { other.ResetKey(); }
    }

    [Fact]
    public void AfterAResetTheOldBytesNoLongerDecrypt()
    {
        if (OperatingSystem.IsWindows() || !_storage.IsEncryptionAvailable) return;
        var encrypted = _storage.Encrypt("secret"u8);

        _storage.ResetKey();

        Assert.Throws<CryptographicException>(() => _storage.Decrypt(encrypted));
        Assert.Equal("new"u8.ToArray(), _storage.Decrypt(_storage.Encrypt("new"u8)));
    }
}
