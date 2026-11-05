// The page's `window.print()` opens the system print dialog through the native side. Frames
// without `window.ipc` keep the original. See `PRINT_SCRIPT` in webview_ext.rs.
(() => {
  const print = window.print;
  window.print = function () {
    if (window.ipc) window.ipc.postMessage('__rustino:print');
    else print.call(window);
  };
})();
