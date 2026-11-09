namespace Rustino.NET.Tests;

/// <summary>Global shortcuts, child windows and monitor changes without the native window.</summary>
public class DesktopTests
{
    private static MonitorInfo Monitor(string name, int x = 0, double scale = 1, bool primary = false) =>
        new() { Name = name, X = x, Width = 1920, Height = 1080, ScaleFactor = scale, IsPrimary = primary };

    [Fact]
    public void SameMonitorsAreNoChange()
    {
        Assert.Null(MonitorsChangedEventArgs.Compare([Monitor("A", primary: true), Monitor("B", 1920)],
            [Monitor("A", primary: true), Monitor("B", 1920)]));
    }

    [Fact]
    public void MonitorsAreMatchedByName()
    {
        var args = MonitorsChangedEventArgs.Compare([Monitor("A", primary: true), Monitor("B", 1920)],
            [Monitor("A", scale: 2, primary: true), Monitor("C", -1920)]);
        Assert.NotNull(args);
        Assert.Equal(["C"], args.Added.Select(m => m.Name));
        Assert.Equal(["B"], args.Removed.Select(m => m.Name));
        Assert.Equal(["A"], args.Changed.Select(m => m.Name));
        Assert.Equal(["A", "C"], args.Monitors.Select(m => m.Name));
    }

    [Fact]
    public void MonitorsOfTheSameModelAreToldApart()
    {
        MonitorInfo[] twins = [Monitor("Dell U2720Q", primary: true), Monitor("Dell U2720Q", 1920)];
        Assert.Null(MonitorsChangedEventArgs.Compare(twins, [twins[1], twins[0]]));

        var unplugged = MonitorsChangedEventArgs.Compare(twins, [Monitor("Dell U2720Q", primary: true)]);
        Assert.NotNull(unplugged);
        Assert.Equal([1920], unplugged.Removed.Select(m => m.X));
        Assert.Empty(unplugged.Added);
        Assert.Empty(unplugged.Changed);

        var plugged = MonitorsChangedEventArgs.Compare([twins[0]], twins);
        Assert.NotNull(plugged);
        Assert.Equal([1920], plugged.Added.Select(m => m.X));
        Assert.Empty(plugged.Removed);
    }

    [Fact]
    public void ShortcutsBeforeTheWindowRunsAreRemembered()
    {
        using var window = new RustinoWindow();
        Assert.False(window.UnregisterGlobalShortcut("CmdOrCtrl+Alt+K"));
        Assert.False(window.UnregisterGlobalShortcut("CmdOrCtrl+Nope"));
        Assert.True(window.RegisterGlobalShortcut("CmdOrCtrl+Alt+K"));
        Assert.True(window.UnregisterGlobalShortcut("cmdorctrl+alt+k"));
        Assert.False(window.UnregisterGlobalShortcut("CmdOrCtrl+Alt+K"));
        Assert.True(window.RegisterGlobalShortcut("CmdOrCtrl+Alt+K"));
        window.UnregisterAllGlobalShortcuts();
        Assert.False(window.UnregisterGlobalShortcut("CmdOrCtrl+Alt+K"));
    }

    [Fact]
    public void AWindowIsNotItsOwnParent()
    {
        using var window = new RustinoWindow();
        Assert.Throws<ArgumentException>(() => window.SetParent(window));
    }

    [Fact]
    public void ShortcutsNeedAnAccelerator()
    {
        using var window = new RustinoWindow();
        Assert.Throws<ArgumentException>(() => window.RegisterGlobalShortcut(" "));
        Assert.False(window.IsGlobalShortcutRegistered("CmdOrCtrl+K"));
        Assert.Null(window.GetCursorPosition());
    }
}
