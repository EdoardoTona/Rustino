namespace Rustino.NET;

public class RustinoSplashscreen : IDisposable
{
    private readonly object _lock = new();
    private IntPtr _nativeHandle;
    private string? _tempFilePath;
    private bool _closed;
    private bool _disposed;

    public RustinoSplashscreen(string imagePath, int width = 400, int height = 300)
    {
        if (string.IsNullOrWhiteSpace(imagePath))
        {
            throw new ArgumentException("Image path cannot be null or empty", nameof(imagePath));
        }

        if (!File.Exists(imagePath))
        {
            throw new FileNotFoundException($"Splash image not found: {imagePath}", imagePath);
        }

        if (width <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(width), "Width must be positive");
        }

        if (height <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(height), "Height must be positive");
        }

        NativeLibraryResolver.EnsureRegistered();
        _nativeHandle = RustinoDllImports.rustino_splash_create(imagePath, width, height);

        if (_nativeHandle == IntPtr.Zero)
        {
            throw new InvalidOperationException("Failed to create splashscreen window");
        }
    }

    public static RustinoSplashscreen FromImage(Stream imageStream, int width = 400, int height = 300)
    {
        if (imageStream == null)
        {
            throw new ArgumentNullException(nameof(imageStream));
        }

        if (!imageStream.CanRead)
        {
            throw new ArgumentException("Stream must be readable", nameof(imageStream));
        }

        // Write stream to a temporary file
        var tempPath = Path.Combine(Path.GetTempPath(), $"rustino_splash_{Guid.NewGuid():N}.png");

        try
        {
            using (var fileStream = File.Create(tempPath))
            {
                imageStream.CopyTo(fileStream);
            }

            var splash = new RustinoSplashscreen(tempPath, width, height);
            splash._tempFilePath = tempPath;
            return splash;
        }
        catch
        {
            // Clean up temp file on failure
            try
            {
                if (File.Exists(tempPath))
                {
                    File.Delete(tempPath);
                }
            }
            catch
            {
                // Ignore cleanup errors
            }

            throw;
        }
    }

    // Hides the splashscreen; Dispose() still frees it.
    public void Close()
    {
        lock (_lock)
        {
            if (_closed || _nativeHandle == IntPtr.Zero)
                return;
            _closed = true;
            RustinoDllImports.rustino_splash_close(_nativeHandle);
        }
    }

    public void Dispose()
    {
        Dispose(disposing: true);
        GC.SuppressFinalize(this);
    }

    protected virtual void Dispose(bool disposing)
    {
        lock (_lock)
        {
            if (_disposed)
                return;
            _disposed = true;
            _closed = true;

            if (_nativeHandle != IntPtr.Zero)
            {
                RustinoDllImports.rustino_splash_dtor(_nativeHandle);
                _nativeHandle = IntPtr.Zero;
            }

            // Clean up temporary file if one was created
            if (_tempFilePath != null)
            {
                try
                {
                    if (File.Exists(_tempFilePath))
                    {
                        File.Delete(_tempFilePath);
                    }
                }
                catch
                {
                    // Ignore cleanup errors
                }

                _tempFilePath = null;
            }
        }
    }

    ~RustinoSplashscreen()
    {
        Dispose(disposing: false);
    }
}
