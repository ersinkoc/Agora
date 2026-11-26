// The response headers the hub sends for a path of the web app, read from crates/agora-hub/src/web.rs (its constants, line continuations
// joined), so a browser test served by Node runs under exactly the hub's page policies: the window's for every path, and the plugin
// frame page's own for that one path (ADR-0114 Amendment A). `web.rs` tests the same headers per path.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const rs = readFileSync(join(root, 'crates/agora-hub/src/web.rs'), 'utf8');
function constant(name) {
  const m = new RegExp(`const ${name}: &str = "((?:[^"\\\\]|\\\\.)*)";`, 's').exec(rs);
  if (!m) throw new Error(`web.rs: no constant ${name}`);
  return m[1].replace(/\\\n\s*/g, '');
}
export const APP_CSP = constant('APP_CSP');
export const PLUGIN_FRAME_CSP = constant('PLUGIN_FRAME_CSP');
export const PLUGIN_FRAME_PATH = constant('PLUGIN_FRAME_PATH');
if (!/X-Frame-Options: \{frame_options\}/.test(rs) || !/Page::PluginFrame => \("SAMEORIGIN", PLUGIN_FRAME_CSP\)/.test(rs) || !/Page::App => \("DENY", APP_CSP\)/.test(rs)) {
  throw new Error('web.rs: the per-path headers are not the ones this helper reproduces');
}

/** The headers of a 200 answer for `path` (without its query); `framePage` is false when the hub has no such file (a 404). */
export function hubHeaders(path, framePage = true) {
  const frame = framePage && path.split('?')[0] === PLUGIN_FRAME_PATH;
  return {
    'x-content-type-options': 'nosniff',
    'x-frame-options': frame ? 'SAMEORIGIN' : 'DENY',
    'referrer-policy': 'no-referrer',
    'content-security-policy': frame ? PLUGIN_FRAME_CSP : APP_CSP,
    'cache-control': 'no-cache',
  };
}
