using System.Runtime.ExceptionServices;

namespace Rustino.NET;

internal interface ICompletable
{
    void Complete();
}

internal sealed class EventObservable<T> : IObservable<T>, ICompletable
{
    private readonly object _lock = new();
    private readonly List<IObserver<T>> _observers = new();
    private bool _completed;

    public IDisposable Subscribe(IObserver<T> observer)
    {
        lock (_lock)
        {
            if (_completed)
            {
                observer.OnCompleted();
                return EmptyDisposable.Instance;
            }
            _observers.Add(observer);
        }
        return new Unsubscriber(this, observer);
    }

    // Every observer gets the value even when one throws; the exceptions are thrown afterwards
    internal void Emit(T value)
    {
        IObserver<T>[] snapshot;
        lock (_lock)
        {
            if (_completed) return;
            snapshot = _observers.ToArray();
        }
        List<Exception>? errors = null;
        foreach (var observer in snapshot)
        {
            try { observer.OnNext(value); }
            catch (Exception exception) { (errors ??= []).Add(exception); }
        }
        Throw(errors);
    }

    // Every observer completes even when one throws; the exceptions are thrown afterwards
    public void Complete()
    {
        IObserver<T>[] snapshot;
        lock (_lock)
        {
            if (_completed) return;
            _completed = true;
            snapshot = _observers.ToArray();
            _observers.Clear();
        }
        List<Exception>? errors = null;
        foreach (var observer in snapshot)
        {
            try { observer.OnCompleted(); }
            catch (Exception exception) { (errors ??= []).Add(exception); }
        }
        Throw(errors);
    }

    private static void Throw(List<Exception>? errors)
    {
        if (errors is null) return;
        if (errors.Count == 1) ExceptionDispatchInfo.Throw(errors[0]);
        throw new AggregateException(errors);
    }

    private sealed class Unsubscriber(EventObservable<T> parent, IObserver<T> observer) : IDisposable
    {
        public void Dispose()
        {
            lock (parent._lock) { parent._observers.Remove(observer); }
        }
    }

    private sealed class EmptyDisposable : IDisposable
    {
        internal static readonly EmptyDisposable Instance = new();
        public void Dispose() { }
    }
}
