using Rustino.Blazor;
using Rustino.Samples.Blazor;

var builder = RustinoBlazorAppBuilder.CreateDefault(args);
builder.RootComponents.Add<App>("#app");

var app = builder.Build();
app.MainWindow
    .SetTitle("Rustino.Blazor Sample")
    .SetSize(960, 760)
    .SetDevToolsEnabled(true);

app.Run();
