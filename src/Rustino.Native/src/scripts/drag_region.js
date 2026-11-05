// Initialization script of every window: window drag and resize from the page, and the window
// controls of the overlay title bar. See `DRAG_REGION_SCRIPT` in window_ext.rs.
(() => {
  if (window.__rustino_window) return;

  const post = (message) => window.ipc && window.ipc.postMessage('__rustino:' + message);

  // ---------------------------------------------------------------------------------------------
  // Drag and resize
  // ---------------------------------------------------------------------------------------------

  const DRAG_ATTRIBUTE = 'data-rustino-drag-region';

  /** Squared distance, in screen pixels, the mouse moves before a press becomes a drag. */
  const DRAG_THRESHOLD = 4;

  const RESIZE_CURSORS = {
    n: 'ns-resize',
    s: 'ns-resize',
    e: 'ew-resize',
    w: 'ew-resize',
    ne: 'nesw-resize',
    sw: 'nesw-resize',
    nw: 'nwse-resize',
    se: 'nwse-resize',
  };

  /** Width of the resize edges in CSS pixels, 0 when the page doesn't resize the window. */
  let border = 0;
  /** Screen position of a press on a drag region, until the drag starts. */
  let pressed = null;
  let cursorStyle = null;

  /** The edge under the mouse ('n', 'se', ...), or null. */
  const edgeAt = (e) => {
    if (!border) return null;
    const v = e.clientY < border ? 'n' : e.clientY >= innerHeight - border ? 's' : '';
    const h = e.clientX < border ? 'w' : e.clientX >= innerWidth - border ? 'e' : '';
    return v + h || null;
  };

  /** Forces the cursor on the whole page, or restores the page's cursors with null. */
  const setCursor = (cursor) => {
    if (!cursor) {
      if (cursorStyle) cursorStyle.remove();
      return;
    }
    cursorStyle = cursorStyle || document.createElement('style');
    cursorStyle.textContent = `* { cursor: ${cursor} !important }`;
    if (!cursorStyle.isConnected) document.documentElement.appendChild(cursorStyle);
  };

  /** The element itself, not its children, so buttons in a title bar stay clickable. */
  const isDragRegion = (el) =>
    el instanceof Element && el.hasAttribute(DRAG_ATTRIBUTE) && el.getAttribute(DRAG_ATTRIBUTE) !== 'false';

  addEventListener('mousemove', (e) => {
    if (pressed) {
      const dx = e.screenX - pressed.x;
      const dy = e.screenY - pressed.y;
      if (!(e.buttons & 1)) {
        pressed = null;
      } else if (dx * dx + dy * dy > DRAG_THRESHOLD) {
        pressed = null;
        post('drag');
      }
      return;
    }
    setCursor(RESIZE_CURSORS[edgeAt(e)]);
  }, true);

  addEventListener('mouseout', (e) => {
    if (!e.relatedTarget) setCursor(null);
  }, true);

  addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    const edge = edgeAt(e);
    if (edge) {
      e.preventDefault();
      e.stopImmediatePropagation();
      post('resize:' + edge);
    } else if (isDragRegion(e.target)) {
      e.preventDefault();
      // The drag starts when the mouse moves, so double clicks still reach the page
      if (e.detail === 2) {
        pressed = null;
        post('maximize');
      } else {
        pressed = { x: e.screenX, y: e.screenY };
      }
    }
  }, true);

  addEventListener('mouseup', () => {
    pressed = null;
  }, true);

  // ---------------------------------------------------------------------------------------------
  // Window controls of the overlay title bar
  // ---------------------------------------------------------------------------------------------

  /** Segoe Fluent Icons / Segoe MDL2 Assets glyphs (Private Use Area). */
  const GLYPHS = {
    minimize: '\uE921',
    maximize: '\uE922',
    restore: '\uE923',
    close: '\uE8BB',
  };

  /** Contents of a 16x16 SVG. */
  const ICONS = {
    minimize: '<path d="M4 8.5h8"/>',
    maximize: '<rect x="4.5" y="4.5" width="7" height="7" rx="1"/>',
    restore: '<rect x="4.5" y="6.5" width="5" height="5" rx="1"/><path d="M6.5 4.5h5v5"/>',
    close: '<path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/>',
  };

  const LABELS = {
    minimize: 'Minimize',
    maximize: 'Maximize',
    restore: 'Restore',
    close: 'Close',
  };

  // Customization, from the page's CSS:
  // - variables (inherited from <html>, or set on `rustino-window-controls`):
  //   --rustino-window-controls-color, -button-width, -button-height, -button-radius, -gap,
  //   -padding, -icon-size, -font (Windows: the icon font, with the Segoe glyphs), -background,
  //   -hover-background, -active-background, -close-hover-background, -close-hover-color,
  //   -disabled-opacity
  // - parts, for anything else: `rustino-window-controls::part(button)`, `::part(minimize)`,
  //   `::part(maximize)`, `::part(restore)` (maximize of a maximized window), `::part(close)`
  //   and `::part(icon)`; the buttons take :hover, :active, :focus-visible and :disabled.
  // Inside the shadow root the variables fall back to the platform defaults below.

  // `all: unset` on the buttons also drops the focus ring: keyboard users get it back
  const COMMON_CSS = `
    :host {
      position: fixed; top: 0; z-index: 2147483647;
      display: flex; align-items: center;
      gap: var(--rustino-window-controls-gap, var(--gap));
      padding: 0 var(--rustino-window-controls-padding, var(--padding));
      box-sizing: border-box;
      user-select: none;
      color: var(--rustino-window-controls-color, #000);
    }
    @media (prefers-color-scheme: dark) {
      :host { color: var(--rustino-window-controls-color, #fff); }
    }
    button {
      all: unset;
      width: var(--rustino-window-controls-button-width, var(--button-width));
      height: var(--rustino-window-controls-button-height, var(--button-height));
      border-radius: var(--rustino-window-controls-button-radius, var(--button-radius));
      display: flex; align-items: center; justify-content: center;
      background: var(--rustino-window-controls-background, var(--background));
    }
    button:hover { background: var(--rustino-window-controls-hover-background, var(--hover-background)); }
    button:active { background: var(--rustino-window-controls-active-background, var(--active-background)); }
    button:focus-visible { outline: 2px solid currentColor; outline-offset: -2px; }
    button:disabled {
      background: var(--rustino-window-controls-background, var(--background));
      color: inherit;
      opacity: var(--rustino-window-controls-disabled-opacity, .35);
    }
  `;

  /** Drawn controls per platform: default title bar height and style. */
  const STYLES = {
    windows: {
      height: 32,
      css: COMMON_CSS + `
        :host {
          --gap: 0px; --padding: 0px;
          --button-width: 46px; --button-height: 100%; --button-radius: 0;
          --icon-size: 10px;
          --background: transparent;
          --hover-background: rgba(128, 128, 128, .2);
          --active-background: rgba(128, 128, 128, .3);
        }
        .icon {
          font: var(--rustino-window-controls-icon-size, var(--icon-size))/1
            var(--rustino-window-controls-font, 'Segoe Fluent Icons', 'Segoe MDL2 Assets');
        }
        button.close:hover:not(:disabled), button.close:active:not(:disabled) {
          background: var(--rustino-window-controls-close-hover-background, #c42b1c);
          color: var(--rustino-window-controls-close-hover-color, #fff);
        }
      `,
    },
    gnome: {
      height: 40,
      css: COMMON_CSS + `
        :host {
          --gap: 12px; --padding: 8px;
          --button-width: 24px; --button-height: 24px; --button-radius: 50%;
          --icon-size: 16px;
          --background: color-mix(in srgb, currentColor 10%, transparent);
          --hover-background: color-mix(in srgb, currentColor 15%, transparent);
          --active-background: color-mix(in srgb, currentColor 30%, transparent);
        }
        .icon {
          width: var(--rustino-window-controls-icon-size, var(--icon-size));
          height: var(--rustino-window-controls-icon-size, var(--icon-size));
          fill: none; stroke: currentColor; stroke-width: 1.5;
        }
        button.close:hover:not(:disabled) {
          background: var(--rustino-window-controls-close-hover-background,
            var(--rustino-window-controls-hover-background, var(--hover-background)));
          color: var(--rustino-window-controls-close-hover-color, inherit);
        }
        button.close:active:not(:disabled) {
          background: var(--rustino-window-controls-close-hover-background,
            var(--rustino-window-controls-active-background, var(--active-background)));
          color: var(--rustino-window-controls-close-hover-color, inherit);
        }
      `,
    },
  };

  const TITLEBAR_PROPERTIES = [
    '--rustino-titlebar-height',
    '--rustino-titlebar-area-x',
    '--rustino-titlebar-area-y',
    '--rustino-titlebar-area-width',
    '--rustino-titlebar-area-height',
  ];

  /** The shadow hosts of the controls, by side, and their widths in the CSS variables. */
  const hosts = {};
  const hostWidths = {};
  let lastControls = null;

  // The page's CSS can resize the controls after they are drawn (button width, gap, padding,
  // a stylesheet that loads later): the room left for them follows
  const hostResizeObserver = new ResizeObserver(() => {
    const changed = ['left', 'right'].some((side) =>
      hosts[side] && hosts[side].isConnected && hosts[side].offsetWidth !== hostWidths[side]);
    if (changed) renderControls(lastControls);
  });

  const buttonHtml = (button, controls) => {
    const icon = button.kind === 'maximize' && controls.maximized ? 'restore' : button.kind;
    const label = LABELS[icon];
    // The glyphs and the icons are hidden from screen readers: the label names the button
    const content = controls.platform === 'windows'
      ? `<span class="icon" part="icon" aria-hidden="true">${GLYPHS[icon]}</span>`
      : `<svg class="icon" part="icon" viewBox="0 0 16 16" aria-hidden="true">${ICONS[icon]}</svg>`;
    // `restore` is a part of its own: ::part() can't be combined with attribute selectors
    const parts = icon === 'restore' ? 'button maximize restore' : 'button ' + button.kind;
    return `<button type="button" class="${button.kind}" part="${parts}" data-action="${button.kind}"` +
      ` aria-label="${label}" title="${label}"${button.enabled ? '' : ' disabled'}>${content}</button>`;
  };

  const createHost = (side) => {
    const host = document.createElement('rustino-window-controls');
    host.style[side] = '0';
    host.attachShadow({ mode: 'open' }).addEventListener('click', (e) => {
      const button = e.target.closest('button');
      if (button && !button.disabled) post(button.dataset.action);
    });
    // The border box, as offsetWidth: a change of the padding alone keeps the content box
    hostResizeObserver.observe(host, { box: 'border-box' });
    return host;
  };

  const renderControls = (controls) => {
    lastControls = controls;
    const root = document.documentElement;
    // No style for the native controls (macOS), which the page only makes room for: they are
    // measured in points, which the page zoom doesn't scale, and devicePixelRatio is the scale
    // factor of the screen times the zoom
    const style = controls && STYLES[controls.platform];
    const unit = controls && !style ? controls.scale_factor / devicePixelRatio : 1;
    const height = controls && (style ? controls.height || style.height : Math.round(controls.height * unit));
    const widths = {
      left: controls && !style ? Math.ceil(controls.native_left * unit) : 0,
      right: 0,
    };

    for (const side of ['left', 'right']) {
      const buttons = style ? controls[side] : [];
      const property = '--rustino-window-controls-' + side;
      if (!buttons.length) {
        if (hosts[side]) hosts[side].remove();
        if (widths[side]) root.style.setProperty(property, widths[side] + 'px');
        else root.style.removeProperty(property);
        continue;
      }
      const host = hosts[side] = hosts[side] || createHost(side);
      host.shadowRoot.innerHTML = `<style>${style.css}</style>` +
        buttons.map((b) => buttonHtml(b, controls)).join('');
      host.style.color = controls.color || '';
      host.style.height = height + 'px';
      if (!host.isConnected) root.appendChild(host);
      widths[side] = hostWidths[side] = host.offsetWidth;
      root.style.setProperty(property, widths[side] + 'px');
    }

    // Safe area of the title bar: the strip free of window controls, as the titlebar-area-*
    // environment variables of the Window Controls Overlay
    if (!controls) {
      for (const name of TITLEBAR_PROPERTIES) root.style.removeProperty(name);
      return;
    }
    const values = [
      height + 'px',
      widths.left + 'px',
      '0px',
      `calc(100vw - ${widths.left + widths.right}px)`,
      height + 'px',
    ];
    TITLEBAR_PROPERTIES.forEach((name, i) => root.style.setProperty(name, values[i]));
  };

  // The page zoom changes the size of the viewport, and so the size of the native controls
  addEventListener('resize', () => {
    if (lastControls && !STYLES[lastControls.platform]) renderControls(lastControls);
  });

  // ---------------------------------------------------------------------------------------------
  // Native interface
  // ---------------------------------------------------------------------------------------------

  /** Called by the native side with the resize border and the window controls. */
  const setState = (state) => {
    border = state.border;
    if (!border) setCursor(null);
    if (document.documentElement) renderControls(state.controls);
    else addEventListener('DOMContentLoaded', () => renderControls(state.controls), { once: true });
  };

  Object.defineProperty(window, '__rustino_window', { value: Object.freeze({ setState }) });
  post('ready');
})();
