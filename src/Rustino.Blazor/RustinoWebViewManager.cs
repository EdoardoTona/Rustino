using System.Security.Cryptography;
using Microsoft.AspNetCore.Components;
using Microsoft.AspNetCore.Components.Web;
using Microsoft.AspNetCore.Components.WebView;
using Microsoft.Extensions.FileProviders;
using Rustino.NET;

namespace Rustino.Blazor;

internal sealed class RustinoWebViewManager : WebViewManager
{
    // Served through a custom scheme on every platform
    // (on Windows, wry exposes it to WebView2 as http://app.localhost/).
    public static readonly Uri AppBaseUri = new("app://localhost/");

    // blazor.webview.js talks to the host through window.external:
    // - messages to .NET start with a per-app token, kept in this main-frame script: frames of other
    //   origins can reach the native IPC handler (which on Linux reports the main frame's URL as the
    //   source of every message), but can't read the token
    // - host messages are dispatched without a source: postMessage from frames or other windows is ignored
    private static string BridgeScript(string token) => $$"""
        (() => {
            const token = '{{token}}';
            window.external = {
                sendMessage: message => window.ipc.postMessage(token + message),
                receiveMessage: callback => window.addEventListener('message', e => {
                    if (e.source === null) callback(e.data);
                })
            };
        })();
        """;

    private readonly string _token = Convert.ToHexString(RandomNumberGenerator.GetBytes(16));
    private readonly RustinoWindow _window;

    public RustinoWebViewManager(
        RustinoWindow window,
        IServiceProvider provider,
        Dispatcher dispatcher,
        IFileProvider fileProvider,
        JSComponentConfigurationStore jsComponents)
        : base(provider, dispatcher, AppBaseUri, fileProvider, jsComponents, "index.html")
    {
        _window = window
            .AddInitScript(BridgeScript(_token))
            .RegisterCustomSchemeHandler(AppBaseUri.Scheme, HandleWebRequest);
        // WebViewManager also ignores messages whose source is not under AppBaseUri,
        // e.g. from a remote page the webview navigated to
        _window.WebMessageReceivedWithSource += (_, e) =>
        {
            if (e.Message.StartsWith(_token, StringComparison.Ordinal)
                && Uri.TryCreate(e.SourceUrl, UriKind.Absolute, out var source))
                MessageReceived(source, e.Message[_token.Length..]);
        };
    }

    public Stream? HandleWebRequest(object sender, string scheme, string url, out string? contentType)
    {
        url = url.Split('?')[0];
        // Paths without a file extension are app routes: fall back to the host page
        if (TryGetResponseContent(url, !Path.HasExtension(url), out var statusCode, out _, out var content, out var headers)
            && statusCode == 200)
        {
            contentType = headers["Content-Type"];
            return content;
        }
        contentType = null;
        return null;
    }

    // For HttpClient: app files keep the provider's status code (404 when missing) and
    // don't fall back to the host page, which is only meant for navigations
    public bool TryGetAppFile(Uri uri, out int statusCode, out string statusMessage, out Stream content, out IDictionary<string, string> headers) =>
        TryGetResponseContent(uri.GetLeftPart(UriPartial.Path), false, out statusCode, out statusMessage, out content, out headers);

    protected override void NavigateCore(Uri absoluteUri) => _window.Load(absoluteUri);

    protected override void SendMessage(string message) => _window.SendWebMessage(message);
}
