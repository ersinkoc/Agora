// A plugin frame's page (ADR-0114 decision 2 and Amendment A; docs/ux/plugins.md, "The frame"). The page is a file the hub serves beside
// the window's own (`public/plugin-frame.html`), at one path, under a header policy of its own: opaque even when opened on its own
// (`sandbox allow-scripts`), no network at all (WebSocket included), framed only by the hub's own window. It is shown in
// `<iframe sandbox="allow-scripts">` (never `allow-same-origin`, so its origin is opaque and it reaches none of the window's storage,
// its key, its cookies or its connection to the hub; no forms, popups, modals, top navigation or downloads). The window's own policy is
// not loosened: it frames only its own origin (`frame-src 'self'`), and every other page of the hub refuses to be framed.
//
// The page runs only the window's loader (`public/plugin-frame.js`); the plugin's checked files come to it by message and run from
// blobs it makes inside the frame's opaque origin.

/** The attribute every plugin frame has, and nothing more. */
export const FRAME_SANDBOX = 'allow-scripts';
/** Where the hub serves the frame page, beside the window's own files. */
export const FRAME_PATH = 'plugin-frame.html';

/** The frame page's address under the window's base. */
export const frameUrl = (base = '/') => `${base.endsWith('/') ? base : `${base}/`}${FRAME_PATH}`;

/** The header policy the hub sends with the frame page (`crates/agora-hub/src/web.rs` `PLUGIN_FRAME_CSP`; a test holds them equal). */
export const FRAME_POLICY = "sandbox allow-scripts; default-src 'none'; script-src 'self' blob:; style-src 'self' blob: 'unsafe-inline'; img-src blob: data:; font-src blob:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; webrtc 'block'; frame-ancestors 'self'";
