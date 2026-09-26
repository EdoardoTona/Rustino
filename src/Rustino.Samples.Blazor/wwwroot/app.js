window.sample = {
    // .NET → JS
    userAgent: () => navigator.userAgent,

    // JS → .NET: notify the component now and whenever the OS color scheme changes
    watchColorScheme: dotnet => {
        const query = matchMedia('(prefers-color-scheme: dark)');
        const notify = () => dotnet.invokeMethodAsync('OnColorSchemeChanged', query.matches);
        query.addEventListener('change', notify);
        notify();
    }
};
