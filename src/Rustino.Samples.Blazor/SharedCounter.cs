namespace Rustino.Samples.Blazor;

// A singleton: the components of every window share it
public class SharedCounter
{
    private int _value;

    public int Value => _value;

    // Raised on the thread of the window that changed the value
    public event Action? Changed;

    public void Increment()
    {
        Interlocked.Increment(ref _value);
        Changed?.Invoke();
    }
}
