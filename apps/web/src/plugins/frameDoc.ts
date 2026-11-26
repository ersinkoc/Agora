// A plugin frame's page (ADR-0114 decision 2 and its Amendment A; docs/ux/plugins.md, "The frame"). The page is a blob the window makes,
// shown in `<iframe sandbox="allow-scripts">` (never `allow-same-origin`, so its origin is opaque and it reaches none of the window's
// storage, its key, its cookies or its connection to the hub; no forms, popups, modals, top navigation or downloads). Its own page
// policy is the first thing in it. A blob page also keeps the window's policy (both apply), so what runs in the frame must pass both.
//
// The one script the page names is the window's loader (`public/plugin-frame.js`), by its full address; the plugin's files come to it
// by message and run from blobs it makes inside the frame. No network at all: `connect-src 'none'`, WebSocket included, whatever the
// window's own policy allows.

/** The attribute every plugin frame has, and nothing more. */
export const FRAME_SANDBOX = 'allow-scripts';
/** Where the window's loader is served, beside the window's own files. */
export const LOADER_PATH = 'plugin-frame.js';

export function loaderUrl(origin: string, base = '/'): string {
  return new URL(`${base.endsWith('/') ? base : `${base}/`}${LOADER_PATH}`, origin).href;
}

/** The frame's page policy: ADR-0114 decision 2's, with the loader's address added to `script-src` (Amendment A). */
export function framePolicy(loader: string): string {
  return `default-src 'none'; script-src blob: ${loader}; style-src blob: 'unsafe-inline'; img-src blob: data:; font-src blob:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'`;
}

const attr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** The frame's page: its policy first, then the loader; nothing of the plugin's. */
export function frameDocument(loader: string, lang = 'en', dir: 'ltr' | 'rtl' = 'ltr'): string {
  const safeLang = /^[A-Za-z]{2,3}(-[A-Za-z0-9]{1,8})*$/.test(lang) ? lang : 'en';
  return `<!doctype html><html lang="${safeLang}" dir="${dir}"><head>`
    + `<meta http-equiv="Content-Security-Policy" content="${attr(framePolicy(loader))}">`
    + '<meta charset="utf-8"><meta name="referrer" content="no-referrer">'
    + `<script src="${attr(loader)}"></script>`
    + '<style>html,body{margin:0;padding:0;background:transparent}</style>'
    + '</head><body></body></html>';
}
