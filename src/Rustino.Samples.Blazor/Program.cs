using Microsoft.Extensions.DependencyInjection;
using Rustino.Blazor;
using Rustino.Samples.Blazor;

var builder = RustinoBlazorAppBuilder.CreateDefault(args);
builder.RootComponents.Add<App>("#app");
// Shared by the components of every window
builder.Services.AddSingleton<SharedCounter>();

var app = builder.Build();
app.MainWindow
    .SetTitle("Rustino.Blazor Sample")
    .SetSize(960, 760)
    .SetDevToolsEnabled(true);

app.Run();
