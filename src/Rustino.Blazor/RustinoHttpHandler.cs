using System.Net;

namespace Rustino.Blazor;

// Lets HttpClient read app files (e.g. "sample-data/weather.json") as in Blazor WebAssembly;
// any other request goes to the network.
internal sealed class RustinoHttpHandler(RustinoWebViewManager manager) : DelegatingHandler(new HttpClientHandler())
{
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var uri = request.RequestUri!;
        if (!RustinoWebViewManager.AppBaseUri.IsBaseOf(uri))
            return base.SendAsync(request, cancellationToken);

        if (!manager.TryGetAppFile(uri, out var statusCode, out var statusMessage, out var content, out var headers))
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.NotFound) { RequestMessage = request });

        var response = new HttpResponseMessage((HttpStatusCode)statusCode)
        {
            ReasonPhrase = statusMessage,
            Content = new StreamContent(content),
            RequestMessage = request
        };
        foreach (var (name, value) in headers)
        {
            if (!response.Content.Headers.TryAddWithoutValidation(name, value))
                response.Headers.TryAddWithoutValidation(name, value);
        }
        return Task.FromResult(response);
    }
}
