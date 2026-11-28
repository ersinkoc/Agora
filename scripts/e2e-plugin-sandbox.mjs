// End-to-end check of sandboxed plugin panels (ADR-0114 build step 2, its Verification list, and Amendment A) in a real Chromium or
// WebKit (the engine of the desktop app's webview on macOS and Linux), under
// the headers the hub really sends per path (scripts/hub-headers.mjs reads them from crates/agora-hub/src/web.rs): the window's own
// policy for every page, and the plugin frame page's own for `/plugin-frame.html`. Each probe is a plugin of its own, loaded in a fresh
// window, and each must fail: storage (IndexedDB, localStorage, cookies), the network (fetch to the hub, to a hub endpoint and to an
// outside host, WebSocket to both, an outside image and font), a popup, top navigation, the frame navigating itself away (to an outside
// address and to a blob page it made), a form post, and reading the window's document; and the frame page opened on its own stays
// opaque. Network probes are judged from outside the frame: by the two servers' request logs and the browser's finished requests, never
// by the frame's word alone, and a probe counts only when the frame ran. Then: a granted read answers, one not granted answers "not
// granted", a malformed or oversized message, or a flood, stops the frame without stalling the window, the window's pane bar and marker cannot be covered, `?safe` and the
// safe-mode key load no plugin and the built-in theme, and a package is loaded through the window's own dialog with the person's grant.
//
//   (cd apps/web && npm ci && npm run gen && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-plugin-sandbox.mjs [--engine chromium|webkit]
//
// Needs a Chromium (CHROME=<path>, default: Playwright's cache) or Playwright's WebKit (WEBKIT=<path>, default: Playwright's cache;
// `playwright-core install --with-deps webkit`). Both hosts and, for WebRTC, a STUN and a TURN listener are first shown reachable from a
// plain page, so a refusal inside a frame is the frame's limits. WebKit runs the frame on the window's own thread (T63): there the
// flood budgets hold for the window's pause after the frame's own send loop has ended, and the whole pause is printed beside it.
// Starts two local servers on 127.0.0.1; changes nothing else.
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import dgram from 'node:dgram';
import net from 'node:net';
import { extname, join } from 'node:path';
import { hubHeaders, APP_CSP, PLUGIN_FRAME_CSP } from './hub-headers.mjs';
import { pack } from './pack-plugin.mjs';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'apps/web/dist');
// `--engine chromium` (the default) or `--engine webkit`, the engine of the desktop app's webview on macOS and Linux.
const engineArg = process.argv.indexOf('--engine');
const engine = engineArg > 0 ? process.argv[engineArg + 1] : 'chromium';
if (!['chromium', 'webkit'].includes(engine)) throw new Error(`--engine chromium|webkit, not ${engine}`);
const pw = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const launchOptions = engine === 'webkit'
  ? (process.env.WEBKIT ? { executablePath: process.env.WEBKIT } : {})
  : { executablePath: process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`, args: ['--no-sandbox'] };
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.png': 'image/png' };

console.log(`engine: ${engine}`);
console.log(`window policy: ${APP_CSP}`);
console.log(`frame page policy: ${PLUGIN_FRAME_CSP}`);

const hubLog = [];
const loaderOrigins = [];
const outLog = [];
const hub = createServer((req, res) => {
  hubLog.push(req.url);
  if (req.url?.startsWith('/plugin-frame.js')) loaderOrigins.push(req.headers.origin ?? '');
  const url = (req.url ?? '/').split('?')[0];
  // The hub's own endpoints answer as a careless one would (CORS open), so only the frame's limits can stop a request.
  if (url.startsWith('/probe') || url.startsWith('/api/')) { res.writeHead(200, { ...hubHeaders(url), 'access-control-allow-origin': '*', 'content-type': 'application/json' }).end('{}'); return; }
  // As web.rs `resolve`: `/` and a missing path without an extension are the app shell; a missing file is a 404.
  let file = join(dist, url === '/' ? 'index.html' : url);
  if (!existsSync(file) && !extname(url)) file = join(dist, 'index.html');
  if (!existsSync(file)) { res.writeHead(404, hubHeaders(url, false)).end(); return; }
  res.writeHead(200, { ...hubHeaders(url), 'content-type': types[extname(file)] ?? 'application/octet-stream' }).end(readFileSync(file));
}).listen(0, '127.0.0.1');
// An outside host: another origin that answers everything, CORS open, so only the frame's own limits can stop a request.
const out = createServer((req, res) => { outLog.push(req.url); res.writeHead(200, { 'access-control-allow-origin': '*', 'content-type': 'text/plain' }).end('ok'); }).listen(0, '127.0.0.1');
for (const s of [hub, out]) s.on('upgrade', (req, sock) => { (s === hub ? hubLog : outLog).push(`upgrade ${req.url}`); sock.destroy(); });
await Promise.all([hub, out].map((s) => new Promise((r) => (s.listening ? r() : s.once('listening', r)))));
const HUB = `http://127.0.0.1:${hub.address().port}`;
const OUT = `http://localhost:${out.address().port}`;

// A plugin package of one entry script (the folder format, packed as scripts/pack-plugin.mjs does).
const tmp = mkdtempSync(join(tmpdir(), 'agora-plugin-e2e-'));
const sha = (s) => createHash('sha256').update(s).digest('hex');
function makePackage(id, entry, capabilities) {
  const manifest = { id, name: `Probe ${id}`, version: '1.0.0', author: 'e2e', licence: 'MIT', min_agora: '0.1.0', panels: [{ id: 'main', kind: 'global' }], capabilities, entry: 'panel.js', files: { 'panel.js': sha(entry) } };
  return JSON.stringify({ 'agora-plugin-bundle': 1, files: { 'agora-plugin.json': Buffer.from(JSON.stringify(manifest)).toString('base64'), 'panel.js': Buffer.from(entry).toString('base64') } });
}
const workspace = (panel, extra = []) => JSON.stringify({ version: 1, layout: { version: 1, root: extra.length ? { type: 'split', direction: 'row', ratios: [0.6, 0.4], children: [{ type: 'pane', id: 'p1', panel }, ...extra] } : { type: 'pane', id: 'p1', panel } }, focus: 'p1', touched: true });
const seeded = (id, text, caps, granted = caps, layout = workspace(`plugin.${id}.main`)) => ({
  'agora.plugins.dev.v1': JSON.stringify([{ text }]),
  'agora.plugins.grants.v1': JSON.stringify({ [id]: { version: '1.0.0', seen: caps, granted } }),
  'agora.workspace.v1': layout,
});

const browser = await pw[engine].launch(launchOptions);
console.log(`browser: ${engine} ${browser.version()}`);
// Both hosts are reachable from this engine outside a plugin frame, so a refusal inside one is the frame's limits, not the host.
{
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  await page.goto(`${OUT}/control/out`);
  await page.goto(`${HUB}/api/control`);
  await ctx.close();
  if (!outLog.includes('/control/out') || !hubLog.includes('/api/control')) throw new Error(`${engine} reaches the hub or the outside host not even from a plain page`);
}
let failed = 0;
const results = [];
const check = (ok, what) => { results.push([ok, what]); console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

// A fresh window with this storage; the window's messages from frames are kept for the probes to report through.
async function open(storage, path = '/#/demo', scheme = 'dark') {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: scheme, locale: 'en-US' });
  // A request the browser refused before sending (a page policy's block) is announced and then fails; only one that finished, or one a
  // server saw, left the frame.
  const requests = [];
  const refused = [];
  ctx.on('requestfinished', (r) => requests.push(r.url()));
  ctx.on('requestfailed', (r) => refused.push(`${r.url()} ${r.failure()?.errorText ?? ''}`));
  const pages = [];
  ctx.on('page', (p) => pages.push(p));
  await ctx.addInitScript((kv) => {
    try { localStorage.setItem('agora.attention.mode', 'off'); for (const [k, v] of Object.entries(kv)) localStorage.setItem(k, v); } catch { /* checked below */ }
  }, storage);
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  // When a frame's own send loop ended and how long it ran, as the frame says on the console (not a message to the window).
  const busy = [];
  page.on('console', (m) => { const b = /^agora-e2e-busy (\d+) (\d+)$/.exec(m.text()); if (b) busy.push({ ms: Number(b[1]), end: Number(b[2]) }); });
  await page.goto(HUB + path);
  return { ctx, page, requests, refused, pages, errors, busy };
}
// A probe keeps its word in its own frame (`window.__probe`), and the test reads it from there: a probe never posts to the window (the
// window stops a frame at its first malformed message). The test waits with plain timeouts and reads, never with a polling wait,
// which keeps the frame's timers from running.
async function report(page, name, timeout = 6000) {
  for (const end = Date.now() + timeout; Date.now() < end;) {
    for (const f of page.frames()) {
      if (f === page.mainFrame()) continue;
      const v = await f.evaluate(() => window.__probe).catch(() => undefined);
      if (v !== undefined) return v;
    }
    await page.waitForTimeout(100);
  }
  return undefined;
}

// ---- Each must fail -----------------------------------------------------------------------------------------------------------
const R = (name, body) => `(async function () { var report = function (r) { if (window.__probe === undefined) window.__probe = String(r); };
  try { ${body} } catch (e) { report('blocked: ' + (e && e.name)); } })();`;
const probes = [
  { name: 'indexedDB.open', code: "var r = indexedDB.open('x'); r.onsuccess = function () { report('opened'); }; r.onerror = function () { report('blocked: ' + (r.error && r.error.name)); };" },
  { name: 'localStorage', code: "localStorage.setItem('x', '1'); report('stored: ' + localStorage.getItem('x'));" },
  { name: 'document.cookie', code: "document.cookie = 'a=1'; report('cookie: ' + document.cookie);" },
  { name: 'fetch to the hub', mark: '/probe/fetch-hub', host: 'hub', code: `fetch(${JSON.stringify(`${HUB}/probe/fetch-hub`)}).then(function () { report('fetched'); }, function (e) { report('blocked: ' + e.name); });` },
  { name: 'fetch to a hub endpoint (/api/session)', mark: '/api/session?from=plugin', host: 'hub', code: `fetch(${JSON.stringify(`${HUB}/api/session?from=plugin`)}).then(function () { report('fetched'); }, function (e) { report('blocked: ' + e.name); });` },
  { name: 'fetch to an outside host', mark: '/probe/fetch-out', host: 'out', code: `fetch(${JSON.stringify(`${OUT}/probe/fetch-out`)}).then(function () { report('fetched'); }, function (e) { report('blocked: ' + e.name); });` },
  { name: 'new WebSocket to the hub', mark: '/probe/ws-hub', host: 'hub', code: `var w = new WebSocket(${JSON.stringify(`${HUB.replace('http', 'ws')}/probe/ws-hub`)}); w.onopen = function () { report('open'); }; w.onerror = function () { report('blocked: error'); };` },
  { name: 'new WebSocket to an outside host', mark: '/probe/ws-out', host: 'out', code: `var w = new WebSocket(${JSON.stringify(`${OUT.replace('http', 'ws')}/probe/ws-out`)}); w.onopen = function () { report('open'); }; w.onerror = function () { report('blocked: error'); };` },
  { name: 'an outside image', mark: '/probe/img', host: 'out', code: `var i = new Image(); i.onload = function () { report('loaded'); }; i.onerror = function () { report('blocked: error'); }; i.src = ${JSON.stringify(`${OUT}/probe/img.png`)};` },
  { name: 'an outside font', mark: '/probe/font', host: 'out', code: `var s = document.createElement('style'); s.textContent = "@font-face { font-family: probe; src: url(${OUT}/probe/font.woff2); }"; document.head.appendChild(s); document.fonts.load('16px probe').then(function (f) { report(f.length ? 'loaded' : 'blocked: none'); }, function (e) { report('blocked: ' + e.name); });` },
  { name: 'window.open', mark: '/probe/popup', host: 'out', popup: true, code: `var w = window.open(${JSON.stringify(`${OUT}/probe/popup`)}); report(w ? 'opened' : 'blocked: null');` },
  { name: 'top navigation', mark: '/probe/top', host: 'out', top: true, code: `top.location.href = ${JSON.stringify(`${OUT}/probe/top`)}; report('assigned');` },
  { name: 'the frame navigating itself away', mark: '/probe/self', host: 'out', self: true, code: `setTimeout(function () { location.href = ${JSON.stringify(`${OUT}/probe/self`)}; }, 600); report('assigned');` },
  // A blob page the frame makes itself, with no policy of its own, that tries to load from both hosts: the window's `frame-src 'self'`
  // refuses the navigation, and the page would keep the frame's policies anyway (a blob page keeps its maker's).
  { name: 'the frame navigating itself to a blob page it made', mark: '/probe/blobnav', host: 'both', code: `var html = '<!doctype html><img src="${HUB}/probe/blobnav-hub.png"><img src="${OUT}/probe/blobnav-out.png"><link rel="stylesheet" href="${OUT}/probe/blobnav.css"><script>fetch("${OUT}/probe/blobnav-fetch")<\\/script>';
    var u = URL.createObjectURL(new Blob([html], { type: 'text/html' })); setTimeout(function () { location.href = u; }, 600); report('assigned');` },
  { name: 'a form post', mark: '/probe/form', host: 'out', self: true, code: `var f = document.createElement('form'); f.method = 'post'; f.action = ${JSON.stringify(`${OUT}/probe/form`)}; var i = document.createElement('input'); i.name = 'k'; i.value = 'secret'; f.appendChild(i); document.body.appendChild(f); report('submitted'); setTimeout(function () { f.submit(); }, 600);` },
  { name: 'reading parent.document', code: "var d = parent.document; report('read: ' + d.title + ' ' + d.cookie);" },
];
for (const p of probes) {
  const id = `probe-${results.length}`;
  const text = makePackage(id, R(p.name, p.code), []);
  const w = await open(seeded(id, text, []));
  const before = { hub: hubLog.length, out: outLog.length };
  await w.page.waitForSelector('iframe[data-plugin-frame]', { timeout: 15000 });
  const said = await report(w.page, p.name);
  await w.page.waitForTimeout(p.self || p.top || p.popup || p.host === 'both' ? 2000 : 800);
  const log = p.host === 'hub' ? hubLog.slice(before.hub) : p.host === 'both' ? [...hubLog.slice(before.hub), ...outLog.slice(before.out)] : outLog.slice(before.out);
  const left = p.mark ? log.some((u) => u.includes(p.mark)) || w.requests.some((u) => u.includes(p.mark)) : false;
  // The frame must have run (it says something) for a refusal to count: a frame that never ran proves nothing.
  let ok = said !== undefined && !left && (/^blocked/.test(said) || ['assigned', 'submitted'].includes(said));
  // Storage and the window's document: the frame's own answer is the evidence, and it must be a refusal.
  if (!p.mark) ok = /^blocked/.test(said ?? '');
  if (p.popup) ok = ok && w.pages.length === 1 && said === 'blocked: null';
  if (p.top) ok = ok && w.page.url().startsWith(HUB);
  // The frame went nowhere outside: it shows the frame page still, or the browser's own error page for a refused navigation.
  if (p.self) {
    const frame = w.page.frames().find((f) => f !== w.page.mainFrame());
    ok = ok && !!frame && !frame.url().startsWith(OUT);
  }
  const blocked = p.mark ? w.refused.filter((u) => u.includes(p.mark)).map((u) => u.split(' ').pop()) : [];
  check(ok, `from inside a plugin frame, ${p.name} fails (frame said: ${said ?? 'nothing'}; request left: ${left ? 'yes' : 'no'}${blocked.length ? `; browser: ${blocked.join(', ')}` : ''})`);
  await w.ctx.close();
}

// The frame page opened on its own, outside any window: its header's `sandbox` keeps it opaque, so it reaches no storage, and its policy
// keeps it off the network; the loader does nothing there.
{
  const d = await open({}, '/plugin-frame.html');
  const res = await d.page.goto(`${HUB}/plugin-frame.html`);
  const h = res?.headers() ?? {};
  const before = hubLog.length;
  const r = await d.page.evaluate(async () => {
    const t = (f) => { try { f(); return 'allowed'; } catch (e) { return `blocked: ${e.name}`; } };
    return {
      origin: self.origin, localStorage: t(() => localStorage.length), indexedDB: t(() => indexedDB.open('x')), cookie: t(() => document.cookie),
      agora: typeof window.agora, fetch: await fetch('/api/direct').then(() => 'fetched', (e) => `blocked: ${e.name}`),
    };
  });
  await d.page.waitForTimeout(500);
  const left = hubLog.slice(before).some((u) => u.includes('/api/direct'));
  check(h['content-security-policy'] === PLUGIN_FRAME_CSP && h['x-frame-options'] === 'SAMEORIGIN', 'the frame page is served under its own policy (sandbox, no network, framed by the hub only)');
  check(r.origin === 'null' && [r.localStorage, r.indexedDB, r.cookie].every((x) => x.startsWith('blocked')) && r.agora === 'undefined' && r.fetch.startsWith('blocked') && !left,
    `the frame page opened on its own stays opaque and reaches nothing (${JSON.stringify(r)}; request left: ${left ? 'yes' : 'no'})`);
  const app = await d.page.goto(`${HUB}/`);
  const ah = app?.headers() ?? {};
  check(ah['content-security-policy'] === APP_CSP && ah['x-frame-options'] === 'DENY' && !APP_CSP.includes('blob:'), `the window keeps its own policy, framing only its own origin (${APP_CSP})`);
  await d.ctx.close();
}

// The hub refuses a request whose Origin is not its own when it sits behind a proxy (ADR-0067, `proxy.rs`, `Origin: null` included):
// the frame's request for the loader must carry no Origin at all.
check(loaderOrigins.length > 0 && loaderOrigins.every((o) => o === ''), `the frame asks for the window's loader without an Origin header (${loaderOrigins.length} requests, origins: ${JSON.stringify([...new Set(loaderOrigins)])})`);

// ---- WebRTC: the one way out a page policy does not close everywhere -------------------------------------------------------
// Judged by a UDP and a TCP listener outside the frame (STUN and TURN servers a plugin names): nothing may reach them. The loader takes
// the RTC constructors away for good; a frame the plugin makes has an opaque origin of its own, so it cannot hand a fresh one back.
{
  const udpLog = [];
  const tcpLog = [];
  const udp = dgram.createSocket('udp4');
  udp.on('message', (m) => udpLog.push(m.length));
  await new Promise((r) => udp.bind(0, '127.0.0.1', r));
  const tcp = net.createServer((c) => { tcpLog.push('conn'); c.destroy(); }).listen(0, '127.0.0.1');
  await new Promise((r) => tcp.once('listening', r));
  const ice = `[{ urls: 'stun:127.0.0.1:${udp.address().port}' }, { urls: 'turn:127.0.0.1:${tcp.address().port}?transport=tcp', username: 'u', credential: 'p' }]`;
  // The control: a plain page (no loader, no frame) does reach them in this engine, so none reaching them from a frame is the loader's.
  const plain = await browser.newContext();
  const pp = await plain.newPage();
  await pp.goto(`${OUT}/control/webrtc`);
  await pp.evaluate(`(function () { var pc = new RTCPeerConnection({ iceServers: ${ice} }); pc.createDataChannel('x'); pc.createOffer().then(function (o) { return pc.setLocalDescription(o); }); window.__pc = pc; })()`);
  for (let i = 0; i < 50 && (udpLog.length === 0 || tcpLog.length === 0); i++) await pp.waitForTimeout(100);
  const control = { udp: udpLog.length, tcp: tcpLog.length };
  await plain.close();
  udpLog.length = 0;
  tcpLog.length = 0;
  const entry = R('webrtc', `
    var out = {};
    var d = Object.getOwnPropertyDescriptor(window, 'RTCPeerConnection');
    out.descriptor = d ? [typeof d.value, d.writable, d.configurable].join(',') : 'none';
    out.left = Object.getOwnPropertyNames(window).filter(function (k) { return /RTC/.test(k) && typeof window[k] === 'function'; }).join(',') || 'none';
    try { delete window.RTCPeerConnection; window.RTCPeerConnection = 1; out.reassign = typeof window.RTCPeerConnection; } catch (e) { out.reassign = 'blocked: ' + e.name; }
    function attempt(C, name) {
      try { var pc = new C({ iceServers: ${ice} }); pc.createDataChannel('x'); pc.createOffer().then(function (o) { return pc.setLocalDescription(o); }); out[name] = 'constructed'; }
      catch (e) { out[name] = 'blocked: ' + e.name; }
    }
    attempt(window.RTCPeerConnection, 'direct');
    attempt(window.webkitRTCPeerConnection, 'webkit');
    var f = document.createElement('iframe'); document.body.appendChild(f);
    try { attempt(f.contentWindow.RTCPeerConnection, 'blankFrame'); } catch (e) { out.blankFrame = 'blocked: ' + e.name; }
    try { attempt(window[0].RTCPeerConnection, 'indexedFrame'); } catch (e) { out.indexedFrame = 'blocked: ' + e.name; }
    var g = document.createElement('iframe');
    g.srcdoc = '<script src="' + URL.createObjectURL(new Blob(["parent.postMessage({ nested: typeof RTCPeerConnection }, '*')"], { type: 'text/javascript' })) + '"><\\/script>';
    out.nested = 'did not run';
    addEventListener('message', function (e) { if (e.data && 'nested' in e.data) out.nested = 'ran: ' + e.data.nested; });
    document.body.appendChild(g);
    setTimeout(function () { report(JSON.stringify(out)); }, 2500);`);
  const id = 'webrtc';
  const w = await open(seeded(id, makePackage(id, entry, []), []));
  await w.page.waitForSelector('iframe[data-plugin-frame]');
  const said = JSON.parse((await report(w.page, 'webrtc', 9000)) ?? '{}');
  await w.page.waitForTimeout(1500);
  check(said.descriptor === 'undefined,false,false' && said.left === 'none' && said.reassign === 'undefined',
    `the RTC constructors are gone for good before the plugin runs (descriptor ${said.descriptor}; left ${said.left}; reassigned ${said.reassign})`);
  check([said.direct, said.webkit, said.blankFrame, said.indexedFrame].every((x) => /^blocked/.test(x ?? '')) && said.nested === 'did not run',
    `no fresh copy from a frame the plugin makes (direct ${said.direct}; webkit ${said.webkit}; blank frame ${said.blankFrame}; window[0] ${said.indexedFrame}; srcdoc script ${said.nested})`);
  check(control.udp + control.tcp > 0 && udpLog.length === 0 && tcpLog.length === 0, `WebRTC from inside a plugin frame reaches no STUN or TURN server (UDP ${udpLog.length}, TCP ${tcpLog.length}; from a plain page: UDP ${control.udp}, TCP ${control.tcp})`);
  await w.ctx.close();
  udp.close();
  tcp.close();
}

// ---- Malformed and oversized messages, and floods --------------------------------------------------------------------------
// Reading a message costs the window as much as the frame put in it (bounded only by memory), and the window's own loader never sends a
// malformed one: the window stops a frame at its first malformed or oversized message, and offers to start it again. Well-formed
// messages over the rate are not read, and after a burst of them the frame is stopped too. Each case is judged by the frame being taken
// away and by the longest pause of the window's own timer.
const stopCases = [
  ['an unknown type', "parent.postMessage({ agora: 1, type: 'open-window', url: 'x' }, '*');", 1500],
  ['an oversized capability name', "parent.postMessage({ agora: 1, type: 'read', id: 99, cap: 'x'.repeat(10000) }, '*');", 1500],
  ['a 16 MB array, then 299 more', "var a = new Uint8Array(16e6); for (var j = 0; j < 300; j++) parent.postMessage(a, '*');", 2000],
  ['a 200,000-field object, then 19 more', "var o = {}; for (var j = 0; j < 200000; j++) o['k' + j] = j; o.agora = 1; o.type = 'read'; for (var k = 0; k < 20; k++) parent.postMessage(o, '*');", 3000],
  ['40 well-formed reads, then 300 16 MB arrays over the rate', "for (var r = 0; r < 40; r++) parent.postMessage({ agora: 1, type: 'read', id: r, cap: 'agents.read' }, '*'); var a = new Uint8Array(16e6); for (var j = 0; j < 300; j++) parent.postMessage(a, '*');", 6000],
];
for (const [name, code, budget] of stopCases) {
  const id = `stop-${stopCases.findIndex((c) => c[0] === name)}`;
  const w = await open(seeded(id, makePackage(id, `(function () { var t0 = performance.now(); ${code} console.log('agora-e2e-busy ' + Math.round(performance.now() - t0) + ' ' + Date.now()); })();`, ['agents.read']), ['agents.read']));
  await w.page.evaluate(() => { window.__gaps = []; let last = Date.now(); setInterval(() => { const n = Date.now(); window.__gaps.push([last, n]); last = n; }, 50); });
  // The frame may be gone before it is ever seen: wait for its pane, then for it to be stopped.
  await w.page.waitForSelector('[data-pane="p1"]');
  let stopped = false;
  for (let i = 0; i < 300 && !stopped; i++) { await w.page.waitForTimeout(100); stopped = (await w.page.locator('[data-plugin-stopped]').count()) > 0; }
  await w.page.waitForTimeout(1500);
  const frames = await w.page.locator('iframe[data-plugin-frame]').count();
  const gaps = await w.page.evaluate(() => window.__gaps);
  const gap = Math.max(0, ...gaps.map(([a, b]) => b - a));
  // Where the frame runs on the window's own thread (WebKit: no process of its own, T63), the window cannot run while the frame's
  // send loop runs, so that much of the pause is the frame's, not the window's reading: the budget holds for what comes after the
  // loop's end. Chromium runs the frame in a process of its own, and the budget holds for the whole pause.
  const loop = w.busy.length ? w.busy[w.busy.length - 1] : undefined;
  const own = engine === 'webkit' && loop ? Math.max(0, ...gaps.map(([a, b]) => b - Math.max(a, loop.end))) : gap;
  let back = true;
  if (name.startsWith('an unknown')) {
    await w.page.getByRole('button', { name: 'Start it again' }).click().catch(() => {});
    back = await w.page.waitForSelector('iframe[data-plugin-frame]', { timeout: 10000 }).then(() => true, () => false);
  }
  const sharing = own === gap ? '' : `, ${own.toFixed(0)} ms of it after the frame's own send loop ended`;
  check(stopped && frames === 0 && back && own < budget, `${name}: the frame is stopped${name.startsWith('an unknown') ? ' and can be started again' : ''}, the window pauses at most ${gap.toFixed(0)} ms${sharing} (budget ${budget}; the frame's send loop ran ${loop ? `${loop.ms} ms` : 'unseen'})`);
  await w.ctx.close();
}

// ---- Grants, messages, containment -----------------------------------------------------------------------------------------
{
  const id = 'reads';
  const entry = R('reads', `
    var names = (await agora.read('agents.read')).agents.map(function (a) { return a.name; });
    var usage = await agora.read('usage.read').then(function () { return 'answered'; }, function (e) { return e.message; });
    var unknown = await agora.read('terminal.read').then(function () { return 'answered'; }, function (e) { return e.message; });
    var cover = document.createElement('div');
    cover.style.cssText = 'position:fixed;inset:-2000px;z-index:2147483647;background:rgb(255,0,0);transform:translateY(-500px) scale(5)';
    document.body.appendChild(cover);
    report(JSON.stringify({ names: names, usage: usage, unknown: unknown }));`);
  const text = makePackage(id, entry, ['agents.read', 'usage.read']);
  const w = await open(seeded(id, text, ['agents.read', 'usage.read'], ['agents.read'], workspace(`plugin.${id}.main`, [{ type: 'pane', id: 'p2', panel: 'approvals' }])));
  await w.page.waitForSelector('iframe[data-plugin-frame]');
  const said = JSON.parse((await report(w.page, 'reads')) ?? '{}');
  check(Array.isArray(said.names) && said.names.length > 0, `a granted read answers (agents.read: ${JSON.stringify(said.names)})`);
  check(said.usage === 'not-granted', `a capability not granted answers "not granted" (usage.read: ${said.usage})`);
  check(said.unknown === 'unknown-capability', `a capability that is none answers "unknown capability" (terminal.read: ${said.unknown})`);
  const box = await w.page.evaluate(() => {
    const r = (el) => { const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; };
    const frame = document.querySelector('iframe[data-plugin-frame]');
    const pane = frame.closest('[data-pane]');
    const mark = document.querySelector('[data-plugin-mark]');
    const bar = [...document.querySelectorAll('.pane-bar')].find((b) => b.contains(mark));
    const other = document.querySelector('[data-pane="p2"]');
    const hit = (el) => { const b = r(el); return document.elementFromPoint(b.cx, b.cy); };
    return { frame: r(frame), pane: r(pane), bar: r(bar), markHit: bar.contains(hit(mark)), markText: mark.textContent, otherHit: other.contains(hit(other)), barHit: bar.contains(hit(bar)), name: bar.querySelector('[data-pane-name]')?.textContent };
  });
  const inside = box.frame.x >= box.pane.x - 0.5 && box.frame.y >= box.pane.y - 0.5 && box.frame.x + box.frame.w <= box.pane.x + box.pane.w + 0.5 && box.frame.y + box.frame.h <= box.pane.y + box.pane.h + 0.5;
  check(inside && box.frame.y >= box.bar.y + box.bar.h - 0.5, 'the frame is drawn inside its pane body, below the window\'s pane bar');
  check(box.markHit && box.barHit && box.markText === 'Plugin development' && box.name === 'Probe reads', `the pane bar with the plugin's name and the marker stays on top of a plugin trying to cover the window ("${box.name}", "${box.markText}")`);
  check(box.otherHit, 'what needs you (the pane beside it) is not covered');
  await w.ctx.close();

  // In a narrow pane the marker keeps its whole width; the name gives way first, then the pane's own buttons, never over the marker.
  const narrow = await open(seeded(id, text, ['agents.read', 'usage.read'], ['agents.read'], JSON.stringify({ version: 1, layout: { version: 1, root: { type: 'split', direction: 'row', ratios: [0.88, 0.12], children: [{ type: 'pane', id: 'p2', panel: 'usage' }, { type: 'pane', id: 'p1', panel: `plugin.${id}.main` }] } }, focus: 'p1', touched: true })));
  await narrow.page.waitForSelector('iframe[data-plugin-frame]');
  const n = await narrow.page.evaluate(() => {
    const mark = document.querySelector('[data-plugin-mark]');
    const bar = [...document.querySelectorAll('.pane-bar')].find((b) => b.contains(mark));
    const m = mark.getBoundingClientRect();
    const b = bar.getBoundingClientRect();
    const at = (x) => mark.contains(document.elementFromPoint(x, m.y + m.height / 2));
    return { inside: m.left >= b.left - 0.5 && m.right <= b.right + 0.5, hits: [at(m.left + 2), at(m.left + m.width / 2), at(m.right - 2)], width: m.width.toFixed(0), bar: b.width.toFixed(0), whole: mark.scrollWidth <= mark.clientWidth + 1 };
  });
  check(n.inside && n.hits.every(Boolean) && n.whole, `in a narrow pane (${n.bar} px) the marker stays whole and uncovered (${n.width} px; hits ${n.hits.join(',')})`);
  await narrow.ctx.close();
}

// ---- Safe mode ----------------------------------------------------------------------------------------------------------------
{
  const id = 'safe';
  const text = makePackage(id, R('safe', "report('ran');"), []);
  const storage = { ...seeded(id, text, []), 'agora.theme': 'dark', 'agora.theme.choice.v1': JSON.stringify({ dark: 'slate', light: 'sand' }) };
  const surface = (page) => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--surface-0').trim());
  const normal = await open(storage);
  await normal.page.waitForSelector('iframe[data-plugin-frame]');
  const themed = await surface(normal.page);
  await normal.ctx.close();
  const safe = await open(storage, '/?safe#/demo');
  await safe.page.waitForSelector('[data-safe-mode]');
  await safe.page.waitForTimeout(1500);
  const frames = await safe.page.locator('iframe').count();
  const plain = await surface(safe.page);
  const placeholder = (await safe.page.locator('[data-pane="p1"]').textContent()) ?? '';
  check(frames === 0 && placeholder.includes('Plugins are off in safe mode') && !placeholder.includes(`plugin.${id}`), `?safe loads no plugin; its pane says plugins are off, in plain words (frames: ${frames})`);
  check(themed !== plain && themed.toLowerCase() === '#0f1419', `?safe shows the built-in theme, not the chosen one (chosen ${themed}, safe ${plain})`);
  await safe.ctx.close();

  const key = await open(storage);
  await key.page.waitForSelector('iframe[data-plugin-frame]');
  await key.page.locator('header').first().click({ position: { x: 5, y: 5 } }).catch(() => {});
  await key.page.keyboard.press('Control+Alt+Shift+KeyS');
  await key.page.waitForURL(/\?safe/, { timeout: 10000 }).catch(() => {});
  await key.page.waitForSelector('[data-safe-mode]', { timeout: 10000 }).catch(() => {});
  check(key.page.url().includes('?safe') && (await key.page.locator('iframe').count()) === 0, `the safe-mode key reopens the window with no plugin (${key.page.url().replace(HUB, '')})`);
  await key.ctx.close();

  // The focus is the person's (ADR-0114 decision 4). A quiet plugin with a text box: its frame is inert until the person clicks into it,
  // and inert again once they click elsewhere, so the key reaches the window. A plugin that keeps taking the focus while not in use is
  // stopped (inert does not stop a frame's own window.focus(), and taking the focus back is a race it wins), so the key reaches the window.
  // Each frame writes down every key that reaches it (read from outside, never sent to the window), so a key lost to it is seen as such.
  const keys = "window.__keys = ''; document.addEventListener('keydown', function (e) { window.__keys += e.code + ';'; }, true);";
  const qid = 'quiet';
  const quiet = makePackage(qid, R('quiet', `var i = document.createElement('input'); document.body.appendChild(i); ${keys} report('ready');`), []);
  const qt = { ...seeded(qid, quiet, [], [], workspace(`plugin.${qid}.main`, [{ type: 'pane', id: 'p2', panel: 'usage' }])) };
  const sid = 'stealer';
  const stealer = makePackage(sid, R('stealer', `var i = document.createElement('input'); document.body.appendChild(i); ${keys} setInterval(function () { window.focus(); i.focus(); }, 20); report('stealing');`), []);
  const st = { ...seeded(sid, stealer, [], [], workspace(`plugin.${sid}.main`, [{ type: 'pane', id: 'p2', panel: 'usage' }])) };
  // E2E_CPU_THROTTLE=<rate> slows a window's CPU as a busy CI runner does (Chromium only), to show timing faults here.
  const throttle = async (w) => {
    if (process.env.E2E_CPU_THROTTLE && engine === 'chromium') await (await w.ctx.newCDPSession(w.page)).send('Emulation.setCPUThrottlingRate', { rate: Number(process.env.E2E_CPU_THROTTLE) });
  };
  const q = await open(qt);
  await throttle(q);
  await q.page.waitForSelector('iframe[data-plugin-frame]');
  await report(q.page, 'quiet');
  await q.page.waitForTimeout(500);
  const untouched = await q.page.evaluate(() => document.activeElement?.tagName);
  const qb = await q.page.locator('iframe[data-plugin-frame]').boundingBox();
  await q.page.mouse.click(qb.x + qb.width / 2, qb.y + qb.height / 2);
  await q.page.waitForTimeout(400);
  const inFrame = await q.page.evaluate(() => document.activeElement?.tagName);
  await q.page.locator('[data-pane="p2"]').click({ position: { x: 20, y: 200 } });
  await q.page.waitForTimeout(600);
  const after = await q.page.evaluate(() => ({ tag: document.activeElement?.tagName, inert: document.querySelector('iframe[data-plugin-frame]')?.inert }));
  const quietFrame = q.page.frames().find((f) => f !== q.page.mainFrame());
  await q.page.keyboard.press('Control+Alt+Shift+KeyS');
  const quietKeys = await quietFrame?.evaluate(() => window.__keys).catch(() => 'none: the window reloaded');
  await q.page.waitForURL(/\?safe/, { timeout: 10000 }).catch(() => {});
  check(untouched !== 'IFRAME' && inFrame === 'IFRAME' && after.tag !== 'IFRAME' && after.inert === true && q.page.url().includes('?safe'),
    `a plugin's frame has the keyboard only while in use: before a click ${untouched}, after clicking in ${inFrame}, after clicking elsewhere ${after.tag} (inert ${after.inert}); the key then opens safe mode [keys in the frame ${JSON.stringify(quietKeys)}; at ${q.page.url().replace(HUB, '')}]`);
  await q.ctx.close();

  const f1 = await open(st);
  await throttle(f1);
  await f1.page.waitForSelector('[data-plugin-stopped]', { timeout: 10000 }).catch(() => {});
  const stoppedText = (await f1.page.locator('[data-plugin-stopped]').textContent().catch(() => '')) ?? '';
  const framesLeft = await f1.page.locator('iframe[data-plugin-frame]').count();
  await f1.page.keyboard.press('Control+Alt+Shift+KeyS');
  await f1.page.waitForURL(/\?safe/, { timeout: 10000 }).catch(() => {});
  check(stoppedText.includes('kept taking the keyboard') && framesLeft === 0 && f1.page.url().includes('?safe'),
    `a plugin taking the focus while not in use is stopped, and the key then opens safe mode (stopped: ${JSON.stringify(stoppedText)}; frames left ${framesLeft}; at ${f1.page.url().replace(HUB, '')})`);
  await f1.ctx.close();
  const f2 = await open(qt);
  await f2.page.waitForSelector('iframe[data-plugin-frame]');
  await f2.page.locator('.pane-bar', { has: f2.page.locator('[data-plugin-mark]') }).locator('button[aria-haspopup]').click();
  await f2.page.getByRole('menuitem', { name: 'Open in safe mode' }).click();
  await f2.page.waitForURL(/\?safe/, { timeout: 10000 }).catch(() => {});
  check(f2.page.url().includes('?safe'), `a plugin pane's menu opens safe mode by the mouse (${f2.page.url().replace(HUB, '')})`);
  // In safe mode the plugins dialog lists what is kept and removes it.
  await f2.page.waitForSelector('[data-safe-mode]');
  await f2.page.keyboard.press('Control+KeyK');
  await f2.page.keyboard.type('Plugins');
  await f2.page.keyboard.press('Enter');
  const row = f2.page.locator(`[data-plugin-row="${qid}"]`);
  await row.waitFor({ timeout: 5000 }).catch(() => {});
  const listed = await row.count();
  await row.getByRole('button', { name: 'Remove' }).click().catch(() => {});
  await f2.page.waitForTimeout(300);
  const kept = await f2.page.evaluate(() => localStorage.getItem('agora.plugins.dev.v1'));
  check(listed === 1 && kept === null, `in safe mode the plugins dialog lists the kept plugin and removes it (listed ${listed}; kept after: ${kept === null ? 'none' : 'some'})`);
  await f2.ctx.close();
}

// ---- Loading through the window's dialog, with the person's grant ---------------------------------------------------------
{
  const file = join(tmp, 'sample.json');
  writeFileSync(file, pack(join(root, 'apps/web/plugins/sample')));
  const w = await open({ 'agora.workspace.v1': workspace('empty', [{ type: 'pane', id: 'p2', panel: 'usage' }]) });
  await w.page.waitForSelector('[data-layout-root]');
  await w.page.keyboard.press('Control+KeyK');
  await w.page.keyboard.type('Plugins');
  await w.page.keyboard.press('Enter');
  await w.page.waitForSelector('[data-plugins-dialog]');
  await w.page.setInputFiles('[data-plugin-file]', file);
  await w.page.waitForSelector('[data-plugin-grants]');
  const words = await w.page.locator('[data-plugin-grants] label').allTextContents();
  check(words.length === 2 && words.every((x) => !/\./.test(x.split(' ')[0] ?? '')), `the install screen lists what it asks to see in plain words (${words.join(' | ')})`);
  await w.page.locator('input[data-cap="agents.read"]').check();
  await w.page.locator('[data-plugin-save]').click();
  await w.page.getByText('Agent list was added', { exact: false }).waitFor();
  await w.page.getByRole('button', { name: 'Done' }).click();
  await w.page.locator('button.ide-chip', { hasText: 'Agent list' }).click();
  const frame = w.page.frameLocator('iframe[data-plugin-frame]');
  await frame.locator('body[data-ready]').waitFor({ timeout: 10000 }).catch(() => {});
  const rows = await frame.locator('li').count();
  const kept = await w.page.evaluate(() => JSON.parse(localStorage.getItem('agora.plugins.grants.v1') ?? '{}'));
  check(rows > 0 && JSON.stringify(kept['sample-agents']?.granted) === '["agents.read"]', `a package loaded from a file shows its panel with what the person allowed (${rows} agents; granted ${JSON.stringify(kept['sample-agents']?.granted)})`);
  check(w.errors.length === 0, `no page errors (${w.errors.join('; ')})`);
  await w.ctx.close();
}

await browser.close();
hub.close();
out.close();
rmSync(tmp, { recursive: true, force: true });
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
