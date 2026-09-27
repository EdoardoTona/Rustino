using System.Buffers;
using System.Buffers.Binary;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.Extensions.Logging;
using Microsoft.Win32.SafeHandles;

namespace Rustino.NET;

internal sealed record SingleInstanceEndpoint(string DisplayId, string LockPath, string PipeName)
{
    private const UnixFileMode PrivateDirectoryMode = UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute;

    public static SingleInstanceEndpoint Create(string appId)
    {
        var scope = OperatingSystem.IsWindows()
            ? $"{WindowsUserSid()}\n{System.Diagnostics.Process.GetCurrentProcess().SessionId}"
            : Environment.UserName;
        var hash = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes($"rustino-single-instance/1\n{appId}\n{scope}"))).ToLowerInvariant();
        var key = hash[..32];

        if (OperatingSystem.IsWindows())
        {
            var local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            if (string.IsNullOrWhiteSpace(local))
                throw new RustinoException("Could not determine the current user's local application data directory.");
            var directory = Path.Combine(local, "Rustino", "SingleInstance");
            Directory.CreateDirectory(directory);
            return new SingleInstanceEndpoint(appId, Path.Combine(directory, $"{key}.lock"), $"Rustino.SingleInstance.{key}");
        }

        var candidates = new List<string>();
        if (OperatingSystem.IsLinux() && !string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable("XDG_RUNTIME_DIR")))
            candidates.Add(Path.Combine(Environment.GetEnvironmentVariable("XDG_RUNTIME_DIR")!, "rustino"));
        var localData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (!string.IsNullOrWhiteSpace(localData))
            candidates.Add(Path.Combine(localData, "Rustino", "SingleInstance"));
        if (OperatingSystem.IsMacOS())
            candidates.Add(Path.Combine(Path.GetTempPath(), "rustino"));
        var userHash = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(Environment.UserName))).ToLowerInvariant()[..12];
        candidates.Add(Path.Combine(Path.GetTempPath(), $"rustino-{userHash}"));

        foreach (var directory in candidates.Distinct(StringComparer.Ordinal))
        {
            try
            {
                Directory.CreateDirectory(directory, PrivateDirectoryMode);
                var info = new DirectoryInfo(directory);
                if (info.LinkTarget is not null)
                    continue;
                if (File.GetUnixFileMode(directory) != PrivateDirectoryMode)
                    File.SetUnixFileMode(directory, PrivateDirectoryMode);
                if (File.GetUnixFileMode(directory) != PrivateDirectoryMode)
                    continue;

                // A shared temp directory can contain a pre-created 0700 directory owned by
                // someone else. Its mode looks private, but this user cannot create the lock.
                var probe = Path.Combine(directory, $".rustino-probe-{Guid.NewGuid():N}");
                using (new FileStream(probe, FileMode.CreateNew, FileAccess.Write, FileShare.None,
                           1, FileOptions.DeleteOnClose)) { }

                var socketPath = Path.Combine(directory, $"{key[..20]}.sock");
                if (Encoding.UTF8.GetByteCount(socketPath) > 100)
                    continue;
                return new SingleInstanceEndpoint(appId, Path.Combine(directory, $"{key}.lock"), socketPath);
            }
            catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
            {
                // Try the private temporary fallback. If all candidates fail, report it below.
            }
        }

        throw new RustinoException("Could not create a private directory for the single-instance endpoint.");
    }

    public bool TryLockUnix(FileStream file)
    {
        if (OperatingSystem.IsWindows()) return true;
        try
        {
            var descriptor = file.SafeFileHandle.DangerousGetHandle().ToInt32();
            var result = OperatingSystem.IsMacOS() ? FlockMac(descriptor, 2 | 4) : FlockLinux(descriptor, 2 | 4);
            if (result == 0) return true;
            var error = Marshal.GetLastPInvokeError();
            if (error == 11 || error == 35) return false; // EWOULDBLOCK (Linux / macOS)
            throw new IOException($"flock failed with errno {error}.");
        }
        catch (DllNotFoundException)
        {
            // FileShare.None still supplies the runtime's cross-process lock on supported Unix runtimes.
            return true;
        }
        catch (EntryPointNotFoundException)
        {
            return true;
        }
    }

    [System.Runtime.Versioning.SupportedOSPlatform("windows")]
    private static string WindowsUserSid()
    {
        try
        {
            return System.Security.Principal.WindowsIdentity.GetCurrent().User?.Value ?? Environment.UserName;
        }
        catch (PlatformNotSupportedException)
        {
            return Environment.UserName;
        }
    }

    [DllImport("libSystem.dylib", EntryPoint = "flock", SetLastError = true)]
    private static extern int FlockMac(int descriptor, int operation);

    [DllImport("libc.so.6", EntryPoint = "flock", SetLastError = true)]
    private static extern int FlockLinux(int descriptor, int operation);
}

internal sealed class SingleInstanceServer(
    SingleInstanceEndpoint endpoint,
    Func<SingleInstanceRequest, bool> enqueue,
    ILogger? logger)
{
    private const int MaximumPayloadSize = 1024 * 1024;
    private readonly CancellationTokenSource _stop = new();
    private NamedPipeServerStream? _listener;
    private Task? _loop;

    public void Start()
    {
        var initial = CreatePipe(first: true);
        _listener = initial;
        _loop = Task.Run(() => AcceptLoopAsync(initial));
    }

    public async Task StopAsync()
    {
        _stop.Cancel();
        try { _listener?.Dispose(); } catch { }
        if (_loop is not null)
        {
            try { await _loop.ConfigureAwait(false); }
            catch (OperationCanceledException) { }
        }
        _stop.Dispose();
    }

    private async Task AcceptLoopAsync(NamedPipeServerStream initial)
    {
        NamedPipeServerStream? listener = initial;
        while (!_stop.IsCancellationRequested)
        {
            if (listener is null)
            {
                await DelayBeforeRetryAsync().ConfigureAwait(false);
                listener = TryCreatePipe(first: false);
                if (listener is null) continue;
                _listener = listener;
            }

            try
            {
                await listener.WaitForConnectionAsync(_stop.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (_stop.IsCancellationRequested) { break; }
            catch (ObjectDisposedException) when (_stop.IsCancellationRequested) { break; }
            catch (Exception exception)
            {
                logger?.LogWarning(exception, "Single-instance listener failed while waiting for a connection.");
                listener.Dispose();
                listener = null;
                continue;
            }

            var connected = listener;
            var next = TryCreatePipe(first: false);
            _listener = next;
            try
            {
                await HandleConnectionAsync(connected, _stop.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            catch (Exception exception)
            {
                logger?.LogWarning(exception, "Single-instance request handling failed.");
            }
            finally
            {
                await connected.DisposeAsync().ConfigureAwait(false);
            }

            listener = next;
        }
        if (listener is not null)
            try { await listener.DisposeAsync().ConfigureAwait(false); } catch { }
    }

    private NamedPipeServerStream? TryCreatePipe(bool first)
    {
        try { return CreatePipe(first); }
        catch (Exception exception)
        {
            logger?.LogWarning(exception, "Could not create the single-instance listener.");
            return null;
        }
    }

    private NamedPipeServerStream CreatePipe(bool first)
    {
        var options = PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly;
        if (first && OperatingSystem.IsWindows())
            options |= PipeOptions.FirstPipeInstance;
        return new NamedPipeServerStream(
            endpoint.PipeName,
            PipeDirection.InOut,
            NamedPipeServerStream.MaxAllowedServerInstances,
            PipeTransmissionMode.Byte,
            options,
            4096,
            4096);
    }

    private async Task HandleConnectionAsync(Stream stream, CancellationToken stopToken)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(stopToken);
        deadline.CancelAfter(TimeSpan.FromSeconds(5));
        var payload = await SingleInstanceProtocol.ReadFrameAsync(stream, MaximumPayloadSize, deadline.Token).ConfigureAwait(false);
        var request = SingleInstanceProtocol.ParseRequest(payload);
        var accepted = request is not null && enqueue(request);
        await SingleInstanceProtocol.WriteResponseAsync(stream, accepted, request is null ? "protocol" : accepted ? null : "busy", deadline.Token)
            .ConfigureAwait(false);
    }

    private async Task DelayBeforeRetryAsync()
    {
        try { await Task.Delay(100, _stop.Token).ConfigureAwait(false); }
        catch (OperationCanceledException) { }
    }
}

internal static class SingleInstanceClient
{
    private const int MaximumPayloadSize = 1024 * 1024;

    public static async Task<ForwardResult> TryForwardAsync(SingleInstanceEndpoint endpoint, SingleInstanceRequest request, TimeSpan timeout)
    {
        using var deadline = new CancellationTokenSource(timeout);
        using var pipe = new NamedPipeClientStream(
            ".", endpoint.PipeName, PipeDirection.InOut, PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
        try
        {
            await pipe.ConnectAsync(deadline.Token).ConfigureAwait(false);
            AllowPrimaryForeground(pipe);
            var payload = SingleInstanceProtocol.SerializeRequest(request);
            if (payload.Length > MaximumPayloadSize)
                return ForwardResult.Rejected;
            await SingleInstanceProtocol.WriteFrameAsync(pipe, payload, deadline.Token).ConfigureAwait(false);
            var response = await SingleInstanceProtocol.ReadFrameAsync(pipe, 4096, deadline.Token).ConfigureAwait(false);
            return SingleInstanceProtocol.ParseResponse(response) ? ForwardResult.Delivered : ForwardResult.Rejected;
        }
        catch (UnauthorizedAccessException)
        {
            return ForwardResult.Rejected;
        }
        catch (OperationCanceledException)
        {
            return ForwardResult.Unavailable;
        }
        catch (IOException)
        {
            return ForwardResult.Unavailable;
        }
        catch (TimeoutException)
        {
            return ForwardResult.Unavailable;
        }
    }

    private static void AllowPrimaryForeground(NamedPipeClientStream pipe)
    {
        if (!OperatingSystem.IsWindows()) return;
        if (GetNamedPipeServerProcessId(pipe.SafePipeHandle, out var serverPid))
            _ = AllowSetForegroundWindow(serverPid);
    }

    [DllImport("kernel32.dll", SetLastError = true, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe, out uint serverProcessId);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AllowSetForegroundWindow(uint processId);
}

internal static class SingleInstanceProtocol
{
    public static byte[] SerializeRequest(SingleInstanceRequest request)
    {
        var buffer = new ArrayBufferWriter<byte>();
        using (var writer = new Utf8JsonWriter(buffer))
        {
            writer.WriteStartObject();
            writer.WriteNumber("v", 1);
            writer.WriteStartArray("args");
            foreach (var argument in request.Args) writer.WriteStringValue(argument);
            writer.WriteEndArray();
            writer.WriteString("cwd", request.WorkingDirectory);
            writer.WriteNumber("pid", request.ProcessId);
            if (request.ActivationToken is null) writer.WriteNull("token");
            else writer.WriteString("token", request.ActivationToken);
            writer.WriteEndObject();
        }
        return buffer.WrittenSpan.ToArray();
    }

    public static SingleInstanceRequest? ParseRequest(ReadOnlyMemory<byte> payload)
    {
        try
        {
            using var document = JsonDocument.Parse(payload);
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("v", out var version) || version.GetInt32() != 1
                || !root.TryGetProperty("args", out var argsElement) || argsElement.ValueKind != JsonValueKind.Array
                || !root.TryGetProperty("cwd", out var cwdElement) || cwdElement.ValueKind != JsonValueKind.String
                || !root.TryGetProperty("pid", out var pidElement) || !pidElement.TryGetInt32(out var processId) || processId <= 0)
                return null;

            var args = new List<string>();
            foreach (var item in argsElement.EnumerateArray())
            {
                if (item.ValueKind != JsonValueKind.String) return null;
                args.Add(item.GetString()!);
            }
            string? token = null;
            if (root.TryGetProperty("token", out var tokenElement))
            {
                if (tokenElement.ValueKind == JsonValueKind.String) token = tokenElement.GetString();
                else if (tokenElement.ValueKind != JsonValueKind.Null) return null;
            }
            return new SingleInstanceRequest(args.ToArray(), cwdElement.GetString()!, processId, token);
        }
        catch (JsonException) { return null; }
        catch (InvalidOperationException) { return null; }
        catch (FormatException) { return null; }
    }

    public static async Task WriteFrameAsync(Stream stream, ReadOnlyMemory<byte> payload, CancellationToken cancellationToken)
    {
        var header = new byte[sizeof(int)];
        BinaryPrimitives.WriteInt32LittleEndian(header, payload.Length);
        await stream.WriteAsync(header, cancellationToken).ConfigureAwait(false);
        await stream.WriteAsync(payload, cancellationToken).ConfigureAwait(false);
        await stream.FlushAsync(cancellationToken).ConfigureAwait(false);
    }

    public static async Task<byte[]> ReadFrameAsync(Stream stream, int maximumLength, CancellationToken cancellationToken)
    {
        var header = new byte[sizeof(int)];
        await stream.ReadExactlyAsync(header, cancellationToken).ConfigureAwait(false);
        var length = BinaryPrimitives.ReadInt32LittleEndian(header);
        if (length <= 0 || length > maximumLength)
            throw new IOException("The single-instance message frame length is invalid.");
        var payload = new byte[length];
        await stream.ReadExactlyAsync(payload, cancellationToken).ConfigureAwait(false);
        return payload;
    }

    public static async Task WriteResponseAsync(Stream stream, bool accepted, string? error, CancellationToken cancellationToken)
    {
        var buffer = new ArrayBufferWriter<byte>();
        using (var writer = new Utf8JsonWriter(buffer))
        {
            writer.WriteStartObject();
            writer.WriteNumber("v", 1);
            writer.WriteBoolean("ok", accepted);
            if (error is not null) writer.WriteString("error", error);
            writer.WriteEndObject();
        }
        await WriteFrameAsync(stream, buffer.WrittenMemory, cancellationToken).ConfigureAwait(false);
    }

    public static bool ParseResponse(ReadOnlyMemory<byte> payload)
    {
        try
        {
            using var document = JsonDocument.Parse(payload);
            var root = document.RootElement;
            return root.ValueKind == JsonValueKind.Object
                && root.TryGetProperty("v", out var version) && version.GetInt32() == 1
                && root.TryGetProperty("ok", out var ok) && ok.ValueKind == JsonValueKind.True;
        }
        catch (JsonException) { return false; }
        catch (InvalidOperationException) { return false; }
    }
}
