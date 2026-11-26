// End-to-end check of sandboxed plugin panels (ADR-0114 build step 2, its Verification list) in a real Chromium, under the page policy
// the hub really sends (read from crates/agora-hub/src/web.rs). Each probe is a plugin of its own, loaded in a fresh window, and each must
// fail: storage (IndexedDB, localStorage, cookies), the network (fetch and WebSocket to the hub and to an outside host, an outside image
// and font), a popup, top navigation, the frame navigating itself away, a form post, and reading the window's document. Network
// probes are judged from outside the frame: by the two servers' request logs and the browser's own request log, never by the frame's
// word alone. Then: a granted read answers, one not granted answers "not granted", unknown and oversized messages are ignored and
// counted, the window's pane bar and marker cannot be covered, `?safe` and the safe-mode key load no plugin and the built-in theme,
// and a package is loaded through the window's own dialog with the person's grant.
//
//   (cd apps/web && npm ci && npm run gen && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-plugin-sandbox.mjs
//   PW_DIR=... node scripts/e2e-plugin-sandbox.mjs --amendment-a   # under the policy ADR-0114 Amendment A proposes (Proposed, not built)
//
// A frame is a blob page, so the window's policy applies inside it as well as its own. Under the hub's policy as it is (`frame-src blob:`
// added, decision 2) the frame loads but no script runs in it, so every probe below reports nothing and the positive checks fail:
// that is the evidence for Amendment A, which adds `blob:` to the window's `script-src`. `--amendment-a` runs the same checks under that
// policy; it changes only this test's server, never the hub.
//
// Needs a Chromium (CHROME=<path>, default: Playwright's cache). Starts two local servers on 127.0.0.1; changes nothing else.
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';
import { pack } from './pack-plugin.mjs';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'apps/web/dist');
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.png': 'image/png' };

// The hub's response headers, as web.rs writes them (its line continuations joined).
const rs = readFileSync(join(root, 'crates/agora-hub/src/web.rs'), 'utf8');
const at = rs.indexOf('Content-Security-Policy: ');
const HUB_CSP = rs.slice(at + 'Content-Security-Policy: '.length, rs.indexOf('\\r\\n', at)).replace(/\\\n\s*/g, '');
if (!HUB_CSP.includes("frame-ancestors 'none'")) throw new Error(`could not read the hub's policy: ${HUB_CSP}`);
const amendment = process.argv.includes('--amendment-a');
const CSP = amendment ? HUB_CSP.replace("default-src 'self';", "default-src 'self'; script-src 'self' blob:;") : HUB_CSP;
if (amendment) console.log('PROPOSED policy (ADR-0114 Amendment A), not what the hub sends today:');
const HEADERS = { 'content-security-policy': CSP, 'x-frame-options': 'DENY', 'x-content-type-options': 'nosniff', 'referrer-policy': 'no-referrer', 'cache-control': 'no-cache' };
console.log(`hub policy: ${CSP}`);

const hubLog = [];
const loaderOrigins = [];
const outLog = [];
const hub = createServer((req, res) => {
  hubLog.push(req.url);
  if (req.url?.startsWith('/plugin-frame.js')) loaderOrigins.push(req.headers.origin ?? '');
  if (req.url?.startsWith('/probe')) { res.writeHead(200, { ...HEADERS, 'access-control-allow-origin': '*' }).end('ok'); return; }
  const path = join(dist, !req.url || req.url.split('?')[0] === '/' ? 'index.html' : req.url.split('?')[0]);
  if (!existsSync(path)) { res.writeHead(404, HEADERS).end(); return; }
  res.writeHead(200, { ...HEADERS, 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
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

const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
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
    window.__probes = [];
    addEventListener('message', (e) => { if (e.data && typeof e.data === 'object' && 'probe' in e.data) window.__probes.push(e.data); });
  }, storage);
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(HUB + path);
  return { ctx, page, requests, refused, pages, errors };
}
const report = (page, name, timeout = 6000) => page.waitForFunction((n) => window.__probes.find((p) => p.probe === n)?.result, name, { timeout }).then((h) => h.jsonValue()).catch(() => undefined);

// ---- Each must fail -----------------------------------------------------------------------------------------------------------
const R = (name, body) => `(async function () { var report = function (r) { parent.postMessage({ probe: ${JSON.stringify(name)}, result: String(r) }, '*'); };
  try { ${body} } catch (e) { report('blocked: ' + (e && e.name)); } })();`;
const probes = [
  { name: 'indexedDB.open', code: "var r = indexedDB.open('x'); r.onsuccess = function () { report('opened'); }; r.onerror = function () { report('blocked: ' + (r.error && r.error.name)); };" },
  { name: 'localStorage', code: "localStorage.setItem('x', '1'); report('stored: ' + localStorage.getItem('x'));" },
  { name: 'document.cookie', code: "document.cookie = 'a=1'; report('cookie: ' + document.cookie);" },
  { name: 'fetch to the hub', mark: '/probe/fetch-hub', host: 'hub', code: `fetch(${JSON.stringify(`${HUB}/probe/fetch-hub`)}).then(function () { report('fetched'); }, function (e) { report('blocked: ' + e.name); });` },
  { name: 'fetch to an outside host', mark: '/probe/fetch-out', host: 'out', code: `fetch(${JSON.stringify(`${OUT}/probe/fetch-out`)}).then(function () { report('fetched'); }, function (e) { report('blocked: ' + e.name); });` },
  { name: 'new WebSocket to the hub', mark: '/probe/ws-hub', host: 'hub', code: `var w = new WebSocket(${JSON.stringify(`${HUB.replace('http', 'ws')}/probe/ws-hub`)}); w.onopen = function () { report('open'); }; w.onerror = function () { report('blocked: error'); };` },
  { name: 'new WebSocket to an outside host', mark: '/probe/ws-out', host: 'out', code: `var w = new WebSocket(${JSON.stringify(`${OUT.replace('http', 'ws')}/probe/ws-out`)}); w.onopen = function () { report('open'); }; w.onerror = function () { report('blocked: error'); };` },
  { name: 'an outside image', mark: '/probe/img', host: 'out', code: `var i = new Image(); i.onload = function () { report('loaded'); }; i.onerror = function () { report('blocked: error'); }; i.src = ${JSON.stringify(`${OUT}/probe/img.png`)};` },
  { name: 'an outside font', mark: '/probe/font', host: 'out', code: `var s = document.createElement('style'); s.textContent = "@font-face { font-family: probe; src: url(${OUT}/probe/font.woff2); }"; document.head.appendChild(s); document.fonts.load('16px probe').then(function (f) { report(f.length ? 'loaded' : 'blocked: none'); }, function (e) { report('blocked: ' + e.name); });` },
  { name: 'window.open', mark: '/probe/popup', host: 'out', popup: true, code: `var w = window.open(${JSON.stringify(`${OUT}/probe/popup`)}); report(w ? 'opened' : 'blocked: null');` },
  { name: 'top navigation', mark: '/probe/top', host: 'out', top: true, code: `top.location.href = ${JSON.stringify(`${OUT}/probe/top`)}; report('assigned');` },
  { name: 'the frame navigating itself away', mark: '/probe/self', host: 'out', self: true, code: `setTimeout(function () { location.href = ${JSON.stringify(`${OUT}/probe/self`)}; }, 50); report('assigned');` },
  // A blob page the frame makes itself, with no policy of its own, that tries to load from both hosts: the window's `frame-src blob:`
  // lets the frame go there, so the frame's own policy must come with it (a blob page keeps its maker's policies).
  { name: 'the frame navigating itself to a blob page it made', mark: '/probe/blobnav', host: 'both', code: `var html = '<!doctype html><img src="${HUB}/probe/blobnav-hub.png"><img src="${OUT}/probe/blobnav-out.png"><link rel="stylesheet" href="${OUT}/probe/blobnav.css"><script>fetch("${OUT}/probe/blobnav-fetch")<\\/script>';
    var u = URL.createObjectURL(new Blob([html], { type: 'text/html' })); setTimeout(function () { location.href = u; }, 50); report('assigned');` },
  { name: 'a form post', mark: '/probe/form', host: 'out', self: true, code: `var f = document.createElement('form'); f.method = 'post'; f.action = ${JSON.stringify(`${OUT}/probe/form`)}; var i = document.createElement('input'); i.name = 'k'; i.value = 'secret'; f.appendChild(i); document.body.appendChild(f); f.submit(); report('submitted');` },
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
  if (p.self) {
    const src = await w.page.locator('iframe[data-plugin-frame]').getAttribute('src');
    ok = ok && /^blob:/.test(src ?? '');
  }
  const blocked = p.mark ? w.refused.filter((u) => u.includes(p.mark)).map((u) => u.split(' ').pop()) : [];
  check(ok, `from inside a plugin frame, ${p.name} fails (frame said: ${said ?? 'nothing'}; request left: ${left ? 'yes' : 'no'}${blocked.length ? `; browser: ${blocked.join(', ')}` : ''})`);
  await w.ctx.close();
}

// The hub refuses a request whose Origin is not its own when it sits behind a proxy (ADR-0067, `proxy.rs`, `Origin: null` included):
// the frame's request for the loader must carry no Origin at all.
check(loaderOrigins.length > 0 && loaderOrigins.every((o) => o === ''), `the frame asks for the window's loader without an Origin header (${loaderOrigins.length} requests, origins: ${JSON.stringify([...new Set(loaderOrigins)])})`);

// ---- Grants, messages, containment -----------------------------------------------------------------------------------------
{
  const id = 'reads';
  const entry = R('reads', `
    var names = (await agora.read('agents.read')).agents.map(function (a) { return a.name; });
    var usage = await agora.read('usage.read').then(function () { return 'answered'; }, function (e) { return e.message; });
    var unknown = await agora.read('terminal.read').then(function () { return 'answered'; }, function (e) { return e.message; });
    parent.postMessage({ agora: 1, type: 'open-window', url: 'x' }, '*');
    parent.postMessage({ agora: 1, type: 'read', id: 99, cap: 'x'.repeat(10000) }, '*');
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
  await w.page.waitForTimeout(1500);
  const dropped = Number(await w.page.locator('iframe[data-plugin-frame]').getAttribute('data-dropped'));
  check(dropped >= 2, `postMessage of an unknown type and an oversized message is ignored and counted (counted: ${dropped})`);
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
  check(frames === 0 && placeholder.includes(`plugin.${id}.main`) && !(await safe.page.evaluate(() => window.__probes.length)), `?safe loads no plugin; its pane shows the placeholder (frames: ${frames})`);
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
