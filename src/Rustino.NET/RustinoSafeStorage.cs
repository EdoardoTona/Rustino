using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;

namespace Rustino.NET;

/// <summary>
/// Encrypts the app's secrets (tokens, passwords) so that only the current user can decrypt them, to keep them in
/// the app's own files: DPAPI on Windows; on macOS and Linux AES-256-GCM with a random key of the app, kept in the
/// login Keychain or in the Secret Service through libsecret (GNOME Keyring, KWallet).
/// </summary>
/// <remarks>
/// The bytes decrypt only for the same user and app id on the same platform (on Windows, on the same machine too).
/// On macOS and Linux <see cref="Encrypt"/> reads the key from the store each time, so a <see cref="ResetKey"/> in
/// another process takes effect; <see cref="Decrypt"/> keeps it in memory. The system may ask the user to allow
/// access or to unlock the keychain, and the call waits for the answer.
/// </remarks>
public sealed class RustinoSafeStorage
{
    private const int Ok = 0;
    private const int InvalidArgument = -1;
    private const int Unavailable = -2;
    private const int Denied = -3;
    private const int InvalidData = -4;

    // The native library may load before any window: from runtimes/<rid>/native too
    static RustinoSafeStorage() => NativeLibraryResolver.EnsureRegistered();

    /// <param name="appId">
    /// Names the key: letters, digits, '.', '-' and '_', such as <c>com.example.MyApp</c>. The same id decrypts in
    /// later runs. macOS: the Keychain item is "&lt;app id&gt; Safe Storage".
    /// </param>
    public RustinoSafeStorage(string appId)
    {
        if (!RustinoAutoStart.IsValidAppId(appId))
            throw new ArgumentException(
                "The app id must have 1 to 128 letters, digits, '.', '-' or '_', and not start with '.'.", nameof(appId));
        AppId = appId;
    }

    /// <summary>The id of the key.</summary>
    public string AppId { get; }

    /// <summary>
    /// Whether <see cref="Encrypt"/> works: false on Linux without a Secret Service (e.g. no keyring daemon) or
    /// when the user refused access to the key. On macOS and Linux it reads the key, creating it the first time.
    /// </summary>
    public bool IsEncryptionAvailable => RustinoDllImports.rustino_safe_storage_available(AppId) == 1;

    /// <summary>Encrypts <paramref name="data"/>.</summary>
    /// <exception cref="RustinoException">No key store, or the user refused access to the key.</exception>
    public byte[] Encrypt(ReadOnlySpan<byte> data)
    {
        var status = RustinoDllImports.rustino_safe_storage_encrypt(
            AppId, in MemoryMarshal.GetReference(data), (nuint)data.Length, out var result, out var length);
        return TakeResult(status, result, length);
    }

    /// <summary>Encrypts the UTF-8 bytes of <paramref name="text"/>.</summary>
    /// <exception cref="RustinoException">No key store, or the user refused access to the key.</exception>
    public byte[] EncryptString(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        var bytes = Encoding.UTF8.GetBytes(text);
        try { return Encrypt(bytes); }
        finally { CryptographicOperations.ZeroMemory(bytes); }
    }

    /// <summary>Decrypts bytes of <see cref="Encrypt"/>.</summary>
    /// <exception cref="CryptographicException">
    /// The bytes aren't of this app and user, were altered, or their key was deleted (<see cref="ResetKey"/>).
    /// </exception>
    /// <exception cref="RustinoException">No key store, or the user refused access to the key.</exception>
    public byte[] Decrypt(ReadOnlySpan<byte> data)
    {
        var status = RustinoDllImports.rustino_safe_storage_decrypt(
            AppId, in MemoryMarshal.GetReference(data), (nuint)data.Length, out var result, out var length);
        return TakeResult(status, result, length);
    }

    /// <summary>Decrypts bytes of <see cref="EncryptString"/>.</summary>
    /// <exception cref="CryptographicException">
    /// The bytes aren't of this app and user, were altered, their key was deleted (<see cref="ResetKey"/>), or
    /// they don't decrypt to UTF-8 text.
    /// </exception>
    /// <exception cref="RustinoException">No key store, or the user refused access to the key.</exception>
    public string DecryptString(ReadOnlySpan<byte> data)
    {
        var bytes = Decrypt(data);
        try { return new UTF8Encoding(false, true).GetString(bytes); }
        catch (DecoderFallbackException) { throw new CryptographicException("The decrypted bytes aren't UTF-8 text."); }
        finally { CryptographicOperations.ZeroMemory(bytes); }
    }

    /// <summary>
    /// Deletes the key of the app, e.g. when the user signs out: what it encrypted no longer decrypts, and the next
    /// <see cref="Encrypt"/> makes a new key. Nothing to delete on Windows, where DPAPI keeps the keys of the user.
    /// </summary>
    /// <exception cref="RustinoException">
    /// No key store, or the user refused access to the key (Linux: also a keyring left locked): the key is still there.
    /// </exception>
    public void ResetKey() => ThrowOnError(RustinoDllImports.rustino_safe_storage_reset(AppId));

    private byte[] TakeResult(int status, IntPtr result, nuint length)
    {
        ThrowOnError(status);
        try
        {
            var bytes = new byte[checked((int)length)];
            Marshal.Copy(result, bytes, 0, bytes.Length);
            return bytes;
        }
        finally
        {
            // Clears the native copy
            RustinoDllImports.rustino_safe_storage_free(result, length);
        }
    }

    private void ThrowOnError(int status)
    {
        switch (status)
        {
            case Ok:
                return;
            case InvalidData:
                throw new CryptographicException($"The data can't be decrypted with the key of '{AppId}'.");
            case Denied:
                throw new RustinoException($"The user refused access to the key of '{AppId}'.");
            case InvalidArgument:
                throw new ArgumentException($"Invalid arguments for the key of '{AppId}'.");
            case Unavailable:
            default:
                throw new RustinoException(
                    $"No key store for '{AppId}': the Keychain on macOS, a Secret Service (libsecret) on Linux.");
        }
    }
}
