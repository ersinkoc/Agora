// The visual pass (roadmap M1.4, ADR-0024; docs/reviews/2026-11-26-visual-pass.md): every main screen of the web window in both themes,
// at a phone width (390 × 844, a touch screen) and a desktop width (1440 × 900), written to docs/screenshots/visual-pass-2026-11/.
// Beside each shot it measures what docs/ux/visual-language.md asks of the screen and prints it (and writes audit.json with --audit):
// controls under 44 px on the phone, text cut off without an ellipsis, a page that scrolls sideways, internal words in visible text or
// in a control's name, and text below 4.5:1 (3:1 when large) on its own background. The measurements are leads for a person who looks
// at the pictures, not verdicts: a cut-off line can be on purpose and a contrast read can miss a gradient.
//
// Two sources:
// - static: the built web app served as the hub serves it (scripts/hub-headers.mjs) with no hub behind it: the demo window (#/demo), the
//   Settings page on demo data (#/gallery/settings), My machines, the gallery's machines-with-a-request and the sign-in page. The clock is
//   pinned (CLOCK), and so are the zone and the language, so these shots are the same on every run on the same fonts.
// - live: a throw-away hub and node (scripts/browser-harness.mjs) with only a shell and agora-fake-agent: the first run, a terminal, a
//   chat agent's conversation, the shared board and Team chat. Their times and folder names come from the run, so they move a little.
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run gen && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots-visual-pass.mjs [--out <dir>] [--only a,b] [--part static|live] [--audit]
//
// Needs git, sh and a Chromium (CHROME=<path>, default: Playwright's cache). The live part uses AGORA_TEST_PORT (default: a free port) and
// a temporary AGORA_HOME; it changes nothing else.
import { createServer } from 'node:http';
import { createServer as netServer } from 'node:net';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';
import { createInterface } from 'node:readline';
import { hubHeaders } from './hub-headers.mjs';
import { pack } from './pack-plugin.mjs';

const root = new URL('..', import.meta.url).pathname;
const web = join(root, 'apps/web');
const dist = join(web, 'dist');
const arg = (name) => { const i = process.argv.indexOf(name); return i > 0 ? process.argv[i + 1] : undefined; };
const out = arg('--out') ?? join(root, 'docs/screenshots/visual-pass-2026-11');
const only = arg('--only')?.split(',');
const part = arg('--part') ?? 'all';
const wantAudit = process.argv.includes('--audit');
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const CLOCK = new Date('2026-11-20T10:00:00Z');
const WIDTHS = { desktop: { width: 1440, height: 900 }, phone: { width: 390, height: 844 } };
const SCHEMES = ['dark', 'light'];
mkdirSync(out, { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), 'vp-'));
const audits = {};
let failed = 0;

// ---- What each shot measures (run in the page) ------------------------------------------------------------------------------
// Internal words (docs/ux/visual-language.md, "UI vocabulary is not engineering vocabulary") as whole words; the reader judges each hit.
const INTERNAL = String.raw`\b(node|nodes|hub|worktree|worktrees|lease|leases|leased|mesh|approval|approvals|blocked|adapter|manifest|ACP|L[0-3]|plugin\.[\w.-]+|agent-\d+)\b`;
function measure({ phone, internal }) {
  const words = new RegExp(internal, 'i');
  const canvas = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
  const rgba = (css) => { canvas.clearRect(0, 0, 1, 1); canvas.fillStyle = 'rgba(0,0,0,0)'; canvas.fillStyle = css; canvas.fillRect(0, 0, 1, 1); const d = canvas.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255]; };
  const over = (top, under) => { const a = top[3]; return [0, 1, 2].map((i) => top[i] * a + under[i] * (1 - a)).concat(1); };
  const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
  const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
  const hidden = (el) => !!el.closest('[inert], [aria-hidden="true"], .sr-only, .xterm-helper-textarea, .xterm-accessibility, details:not([open]) > :not(summary)');
  const shown = (el) => {
    if (hidden(el)) return false;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2 || r.bottom <= 0 || r.right <= 0 || r.top >= innerHeight || r.left >= innerWidth) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0.05;
  };
  const name = (el) => (el.getAttribute('aria-label') || el.getAttribute('title') || el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 48);
  const tag = (el) => `${el.tagName.toLowerCase()}${el.className && typeof el.className === 'string' ? `.${el.className.trim().split(/\s+/).slice(0, 3).join('.')}` : ''}`;
  const result = { sideways: false, small: [], clipped: [], truncated: 0, words: [], contrast: [] };
  const main = document.querySelector('.phone__main') ?? document.scrollingElement;
  result.sideways = document.scrollingElement.scrollWidth > innerWidth + 1 || (main.scrollWidth > main.clientWidth + 1 && getComputedStyle(main).overflowX !== 'hidden');
  // Controls a finger presses, on the phone only (the kit grows to 44 px on a touch screen). Links inside a sentence are exempt (WCAG 2.5.8).
  if (phone) {
    const seen = new Set();
    for (const el of document.querySelectorAll('button, a[href], select, input:not([type=hidden]), textarea, summary, [role=button], [role=tab], [role=menuitem], [role=menuitemradio], [role=radio], [role=switch], [role=option], [role=checkbox]')) {
      if (!shown(el) || el.closest('.xterm')) continue;
      if (el.tagName === 'A' && el.closest('p, li') && getComputedStyle(el).display === 'inline') continue;
      // A visually hidden input whose label is the target: judge the label instead.
      const target = (el.matches('input[type=radio], input[type=checkbox]') && el.closest('label')) || el;
      const r = target.getBoundingClientRect();
      if ((r.width < 43.5 || r.height < 43.5) && !seen.has(target)) { seen.add(target); result.small.push(`${name(target) || tag(target)} ${Math.round(r.width)}×${Math.round(r.height)}`); }
    }
  }
  const textEls = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const el = n.parentElement;
    if (!el || !n.textContent.trim() || el.closest('script, style, .xterm, canvas, iframe') || textEls.at(-1) === el) continue;
    if (shown(el)) textEls.push(el);
  }
  for (const el of textEls) {
    const text = el.textContent.replace(/\s+/g, ' ').trim();
    // Cut off: wider than its box, not scrolled and no ellipsis (the kit's `truncate` is on purpose and only counted).
    for (const box of [el, el.parentElement]) {
      if (!box || box === document.body) continue;
      const s = getComputedStyle(box);
      if (box.scrollWidth > box.clientWidth + 1 && ['hidden', 'clip'].includes(s.overflowX)) {
        if (s.textOverflow === 'ellipsis') result.truncated++;
        else if (!result.clipped.some((c) => c.startsWith(text.slice(0, 40)))) result.clipped.push(`${text.slice(0, 60)} (${tag(box)} ${box.scrollWidth}>${box.clientWidth})`);
        break;
      }
    }
    if (words.test(text) && !el.closest('pre, code, .codebox, [data-diff], [data-file-view], .hljs, textarea, input')) result.words.push(text.slice(0, 80));
    // Contrast against what is under the text: the backgrounds of its ancestors, composed; a gradient or an image is skipped.
    if (el.closest(':disabled, [aria-disabled="true"]')) continue;
    const layers = [];
    let opacity = 1;
    let unsure = false;
    for (let a = el; a; a = a.parentElement) {
      const s = getComputedStyle(a);
      opacity *= Number(s.opacity);
      if (s.backgroundImage !== 'none') { unsure = true; break; }
      const bg = rgba(s.backgroundColor);
      if (bg[3] > 0) layers.push(bg);
      if (bg[3] >= 1) break;
    }
    if (unsure) continue;
    let bg = [255, 255, 255, 1];
    for (const l of layers.reverse()) bg = over(l, bg);
    const s = getComputedStyle(el);
    const fg = rgba(s.color);
    let ink = over(fg, bg);
    if (opacity < 1) ink = over([...ink.slice(0, 3), opacity], bg);
    const size = parseFloat(s.fontSize);
    const large = size >= 24 || (size >= 18.6 && Number(s.fontWeight) >= 700);
    const r = ratio(ink, bg);
    const need = large ? 3 : 4.5;
    if (r < need - 0.05 && !result.contrast.some((c) => c.text === text.slice(0, 40))) result.contrast.push({ text: text.slice(0, 40), ratio: Math.round(r * 100) / 100, need, ink: s.color, on: `rgb(${bg.slice(0, 3).map(Math.round).join(',')})`, el: tag(el) });
  }
  for (const el of document.querySelectorAll('[aria-label], [title], [placeholder]')) {
    if (!shown(el)) continue;
    for (const a of ['aria-label', 'title', 'placeholder']) { const v = el.getAttribute(a); if (v && words.test(v) && !result.words.includes(`[${a}] ${v}`)) result.words.push(`[${a}] ${v.slice(0, 80)}`); }
  }
  return result;
}

// A page is shot once it has settled (as scripts/visual-regression.mjs): fonts loaded, no dwell running, the DOM quiet for a moment.
async function settle(page, limit = 12_000) {
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(() => { window.__vpLast = performance.now(); new MutationObserver(() => { window.__vpLast = performance.now(); }).observe(document, { subtree: true, childList: true, attributes: true, characterData: true }); });
  const start = Date.now();
  for (;;) {
    await page.waitForTimeout(150);
    const s = await page.evaluate(() => ({ quiet: performance.now() - window.__vpLast, dwelling: !!document.querySelector('.attention-card [aria-disabled="true"], [id^="skip-wait-"]') }));
    if (s.quiet >= 600 && !s.dwelling) return;
    if (Date.now() - start > limit) return;
  }
}

async function shoot(page, name, width, scheme, { element } = {}) {
  await settle(page);
  const file = join(out, `${name}-${width}-${scheme}.png`);
  await page.addStyleTag({ content: '*, *::before, *::after { caret-color: transparent !important; }' });
  if (element) await page.locator(element).first().screenshot({ path: file, animations: 'disabled' });
  else await page.screenshot({ path: file, animations: 'disabled' });
  const m = await page.evaluate(measure, { phone: width === 'phone', internal: INTERNAL });
  audits[`${name}-${width}-${scheme}`] = m;
  const notes = [m.sideways && 'SIDEWAYS', m.small.length && `small ${m.small.length}`, m.clipped.length && `clipped ${m.clipped.length}`, m.words.length && `words ${m.words.length}`, m.contrast.length && `contrast ${m.contrast.length}`].filter(Boolean);
  console.log(`shot ${name}-${width}-${scheme}${notes.length ? `  [${notes.join(', ')}]` : ''}`);
  if (process.env.VP_VERBOSE) {
    for (const s of m.small) console.log(`   small: ${s}`);
    for (const s of m.clipped) console.log(`   clipped: ${s}`);
    for (const s of m.words) console.log(`   word: ${s}`);
    for (const c of m.contrast) console.log(`   contrast ${c.ratio}<${c.need}: "${c.text}" ${c.ink} on ${c.on} ${c.el}`);
  }
}

// ---- Static part: the demo window and pages with no hub ---------------------------------------------------------------------
const pane = (id, panel, agent) => ({ type: 'pane', id, panel, ...(agent ? { bind: { agent } } : {}) });
const split = (ratios, ...children) => ({ type: 'split', direction: 'row', ratios, children });
const layout = (rootNode, focus = 'p1') => JSON.stringify({ version: 1, layout: { version: 1, root: rootNode }, focus, touched: true });
const samplePlugin = () => pack(join(root, 'apps/web/plugins/sample'));
const pluginStorage = () => ({
  'agora.plugins.dev.v1': JSON.stringify([{ text: samplePlugin() }]),
  'agora.plugins.grants.v1': JSON.stringify({ 'sample-agents': { version: '0.1.0', seen: ['agents.read', 'theme.read'], granted: ['agents.read', 'theme.read'] } }),
  'agora.workspace.v1': layout(split([0.6, 0.4], pane('p1', 'plugin.sample-agents.list'), pane('p2', 'usage'))),
});
const palette = async (page, words) => { await page.keyboard.press('Control+k'); await page.waitForSelector('[role=dialog] input, [role=combobox]'); if (words) await page.keyboard.type(words); };
const openPalette = async (page, width, words) => {
  if (width === 'phone') { await page.getByRole('button', { name: 'Search', exact: true }).click(); if (words) await page.keyboard.type(words); } else await palette(page, words);
};

const STATIC = [
  { name: 'signin', server: 'dist', path: '/', ready: 'form' },
  { name: 'home', path: '/#/demo', ready: '.fleet-summary' },
  // The "Needs you" card that covers the window when an agent asks (attention.mode left on); on a phone, the request at the bottom.
  { name: 'needs-you-card', path: '/#/demo', loud: true, ready: '.fleet-summary', setup: async (page, width) => { if (width === 'desktop') await page.waitForSelector('.attention-card'); } },
  { name: 'start-agent', path: '/#/demo', ready: '.fleet-summary', setup: async (page, width) => {
    await page.getByRole('button', { name: width === 'phone' ? 'Start' : 'Start an agent', exact: true }).locator('visible=true').first().click();
    await page.waitForSelector('[role=dialog]');
  } },
  // An agent that needs you with its request beside it, and the Needs you list.
  { name: 'approval', path: '/#/demo', phonePath: '/#/agent/a1', ready: '[data-layout-root], .phone', storage: { 'agora.workspace.v1': layout(split([0.6, 0.4], pane('p1', 'agent', 'a1'), pane('p2', 'approvals'))) } },
  { name: 'changes', path: '/#/demo', phonePath: '/#/agent/a1', ready: '[data-layout-root], .phone', storage: { 'agora.workspace.v1': layout(split([0.4, 0.6], pane('p1', 'agent', 'a1'), pane('p2', 'changes', 'a1')), 'p2') },
    setup: async (page, width) => { if (width === 'phone') { await page.getByRole('button', { name: /files? changed/ }).first().click(); await page.waitForSelector('[role=dialog]'); } else await page.waitForSelector('[data-pane="p2"] button, [data-pane="p2"] [role=tree], [data-pane="p2"] pre'); } },
  { name: 'files', path: '/#/demo', phonePath: '/#/agent/a1', ready: '[data-layout-root], .phone', storage: { 'agora.workspace.v1': layout(split([0.5, 0.5], pane('p1', 'files', 'a1'), pane('p2', 'manager', 'a1'))) },
    setup: async (page, width) => {
      if (width === 'phone') { await page.getByRole('button', { name: 'Files', exact: true }).first().click(); await page.waitForSelector('[role=dialog]'); return; }
      // Open one file so the viewer shows code.
      const file = page.locator('[data-pane="p1"]').getByText('auth.rs', { exact: false }).first();
      if (await file.count()) await file.click().catch(() => {});
    } },
  // The phone's More menu (the board, Team chat and Usage), opened from the bottom bar. A phone only: a larger screen has the side panel.
  { name: 'phone-more', widths: ['phone'], path: '/#/demo', ready: '.phone', setup: async (page) => { await page.getByRole('button', { name: 'More', exact: true }).click(); await page.waitForSelector('[role=menu]'); } },
  // Usage as a phone screen of its own (from More), on the demo data.
  { name: 'usage', widths: ['phone'], path: '/#/usage', ready: '.phone h1' },
  { name: 'palette', path: '/#/demo', ready: '.fleet-summary', setup: async (page, width) => { await openPalette(page, width); await page.waitForTimeout(300); } },
  { name: 'settings', path: '/#/gallery/settings', ready: '[data-settings]', context: { permissions: ['notifications'] } },
  { name: 'machines', path: '/#/machines', ready: '.page, main' },
  { name: 'theme-editor', path: '/#/demo', phonePath: '/#/theme', ready: 'section[aria-label="Theme"]', storage: { 'agora.workspace.v1': layout(split([0.5, 0.5], pane('p1', 'fleet'), pane('p2', 'theme')), 'p2') } },
  { name: 'theme-import', path: '/#/demo', phonePath: '/#/theme', ready: 'section[aria-label="Theme"]', storage: { 'agora.workspace.v1': layout(split([0.5, 0.5], pane('p1', 'fleet'), pane('p2', 'theme')), 'p2') },
    setup: async (page) => {
      await page.locator('[data-import-other]').setInputFiles({ name: 'harbour-color-theme.json', mimeType: 'application/octet-stream', buffer: readFileSync(join(root, 'apps/web/src/themes/import/fixtures/harbour-color-theme.json')) });
      await page.waitForSelector('[data-theme-import]');
    } },
  { name: 'plugin-panel', widths: ['desktop'], path: '/#/demo', ready: 'iframe[data-plugin-frame]', storage: pluginStorage(),
    setup: async (page) => { await page.frameLocator('iframe[data-plugin-frame]').locator('body[data-ready]').waitFor({ timeout: 10000 }).catch(() => {}); } },
  // Loading a plugin from a file: the screen that lists what it asks to see, before the person allows it.
  { name: 'plugin-grant', path: '/#/demo', ready: '.fleet-summary', setup: async (page, width) => {
    const file = join(scratch, 'sample.json');
    writeFileSync(file, samplePlugin());
    await openPalette(page, width, 'Plugins');
    await page.keyboard.press('Enter');
    await page.waitForSelector('[data-plugins-dialog]');
    await page.setInputFiles('[data-plugin-file]', file);
    await page.waitForSelector('[data-plugin-grants]');
  } },
  { name: 'safe-mode', path: '/?safe#/demo', ready: '[data-safe-mode]', storage: pluginStorage() },
];

async function staticPart(browser) {
  // The built app as the hub serves it, for the sign-in page (a production build with no hub asks for the code).
  const server = createServer((req, res) => {
    const url = (req.url ?? '/').split('?')[0];
    let file = join(dist, url === '/' ? 'index.html' : url);
    if (!existsSync(file) && !extname(url)) file = join(dist, 'index.html');
    if (!existsSync(file)) { res.writeHead(404, hubHeaders(url, false)).end(); return; }
    const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2' };
    res.writeHead(200, { ...hubHeaders(url), 'content-type': types[extname(file)] ?? 'application/octet-stream' }).end(readFileSync(file));
  }).listen(0, '127.0.0.1');
  await new Promise((r) => server.once('listening', r));
  // The web app's own Vite in development, where the demo data stands in for a hub on every route (an agent's phone view, #/theme).
  const requireWeb = createRequire(join(web, 'package.json'));
  const { createServer: vite } = await import(requireWeb.resolve('vite'));
  const dev = await vite({ root: web, configFile: join(web, 'vite.config.ts'), logLevel: 'error', server: { port: 0, host: '127.0.0.1' } });
  await dev.listen();
  const bases = { dist: `http://127.0.0.1:${server.address().port}`, dev: `http://127.0.0.1:${dev.httpServer.address().port}` };
  try {
    for (const s of STATIC) {
      if (only && !only.includes(s.name)) continue;
      for (const width of s.widths ?? ['desktop', 'phone']) for (const scheme of SCHEMES) {
        const ctx = await browser.newContext({ viewport: WIDTHS[width], colorScheme: scheme, reducedMotion: 'reduce', locale: 'en-US', timezoneId: 'UTC', deviceScaleFactor: 1, ...s.context });
        await ctx.addInitScript(({ loud, kv }) => { try { if (!loud) localStorage.setItem('agora.attention.mode', 'off'); for (const [k, v] of Object.entries(kv)) localStorage.setItem(k, v); } catch { /* shot as it is */ } }, { loud: !!s.loud, kv: s.storage ?? {} });
        const page = await ctx.newPage();
        const errors = [];
        page.on('pageerror', (e) => errors.push(e.message));
        // A phone is a touch screen: the kit grows its controls to 44 px only there.
        if (width === 'phone') await (await ctx.newCDPSession(page)).send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
        await page.clock.setFixedTime(CLOCK);
        try {
          await page.goto(bases[s.server ?? 'dev'] + (width === 'phone' && s.phonePath ? s.phonePath : s.path));
          await page.waitForSelector(s.ready, { timeout: 15000 });
          if (s.setup) await s.setup(page, width);
          await shoot(page, s.name, width, scheme, { element: s.element?.(scheme) });
        } catch (e) { failed++; console.log(`FAIL ${s.name}-${width}-${scheme}: ${e.message.split('\n')[0]}`); }
        if (errors.length) { failed++; console.log(`FAIL ${s.name}-${width}-${scheme}: page errors: ${errors.join(' | ')}`); }
        await ctx.close();
      }
    }
  } finally { server.close(); await dev.close(); }
}

// ---- Live part: a hub, a node, a shell and agora-fake-agent -----------------------------------------------------------------
const freePort = () => new Promise((resolve) => { const s = netServer().listen(0, '127.0.0.1', () => { const { port } = s.address(); s.close(() => resolve(port)); }); });
function mesh(bin, env, socket) {
  const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const pending = new Map();
  let next = 0;
  createInterface({ input: child.stdout }).on('line', (line) => { const reply = JSON.parse(line); const p = pending.get(reply.id); if (!p) return; pending.delete(reply.id); if (reply.error) p.reject(new Error(JSON.stringify(reply.error))); else p.resolve(reply.result); });
  const call = (method, params) => new Promise((resolve, reject) => { const id = ++next; pending.set(id, { resolve, reject }); child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`); });
  return {
    init: () => call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'visual-pass', version: '1' } }),
    tool: async (name, args) => { const r = await call('tools/call', { name, arguments: args }); if (r.isError) throw new Error(JSON.stringify(r)); return JSON.parse(r.content.find((c) => c.type === 'text').text); },
    close: () => child.kill(),
  };
}

async function livePart() {
  process.env.AGORA_TEST_PORT ??= String(await freePort());
  const { startStack } = await import('./browser-harness.mjs');
  const fake = join(root, 'target/debug/agora-fake-agent');
  const stack = await startStack({ viewport: WIDTHS.desktop, hostname: 'laptop' });
  const clients = [];
  const { page, run, bin, env, home, proj, origin, errors } = stack;
  page.setDefaultTimeout(30000);
  const cdp = await page.context().newCDPSession(page);
  const want = (name) => !only || only.includes(name);
  // Each live screen in both widths and both themes, on the one signed-in page: the phone is the same page made narrow and touch.
  const each = async (name, go, widths = ['desktop', 'phone']) => {
    if (!want(name)) return;
    for (const width of widths) {
      await page.setViewportSize(WIDTHS[width]);
      await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: width === 'phone', maxTouchPoints: 5 });
      try {
        await go(width);
        for (const scheme of SCHEMES) { await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' }); await page.waitForTimeout(300); await shoot(page, name, width, scheme); }
      } catch (e) { failed++; console.log(`FAIL ${name}-${width}: ${e.message.split('\n')[0]}`); }
    }
  };
  try {
    await page.evaluate(() => { try { localStorage.setItem('agora.attention.mode', 'off'); } catch { /* shot as it is */ } });
    // The first run: signed in, a machine, no agent yet.
    await each('first-run', async () => { await page.goto(`${origin}/#/`); await page.reload(); await page.waitForSelector('text=Start an agent'); });

    // A shell with a little output, a second shell for the board, and a chat agent (agora-fake-agent over ACP with its settings).
    const id = (o) => o.match(/agent-\d+/)?.[0];
    const shell = id(run(['agent', 'start', 'shell', '--cwd', proj]));
    const helper = id(run(['agent', 'start', 'shell', '--cwd', proj]));
    const argv = ['/bin/sh', '-c', `AGORA_FAKE_ACP_CONFIG=1 exec '${fake}' --acp`];
    writeFileSync(join(home, 'manifests/fake-rich.toml'), `kind = "fake-rich"\ndisplay_name = "Fake chat agent (rich)"\nversion = "1"\nmin_engine = 1\n[structured]\nacp = { argv = ${JSON.stringify(argv)} }\n`);
    const chat = id(run(['agent', 'start', 'fake-rich', '--cwd', proj]));
    const key = (a) => encodeURIComponent(`laptop/${a}`);

    await page.goto(`${origin}/#/agent/${key(shell)}`);
    await page.waitForFunction(() => /\$\s*$/.test((document.querySelector('.xterm-rows')?.innerText ?? '').trim()), null, { timeout: 30000 });
    await page.locator('.xterm').first().click();
    await page.keyboard.type("printf 'README.md\\nsrc/\\n' > list.txt; git status --short; cat list.txt; printf '\\033[32mok\\033[0m 12 tests passed\\n'\n");
    await page.waitForTimeout(800);
    await each('terminal', async () => { await page.goto(`${origin}/#/agent/${key(shell)}`); await page.waitForSelector('.xterm-rows'); await page.waitForTimeout(800); });

    await page.goto(`${origin}/#/agent/${key(chat)}`);
    await page.waitForSelector('form textarea', { timeout: 30000 });
    const say = async (text) => { for (let i = 0; i < 5; i++) { await page.fill('form textarea', text); await page.waitForTimeout(300); if ((await page.inputValue('form textarea')) === text) break; } await page.click('button:has-text("Send")'); };
    await say('rich');
    await page.waitForSelector('text=done', { timeout: 30000 });
    await say('ask');
    await page.getByRole('button', { name: 'Allow', exact: true }).first().waitFor({ timeout: 30000 });
    await each('chat', async () => { await page.goto(`${origin}/#/agent/${key(chat)}`); await page.waitForSelector('[data-plan], [data-steps]'); });

    // The shared board and Team chat, from the two shells' own mesh connections.
    for (const a of [shell, helper]) { const c = mesh(bin, env, join(home, 'm', `${a}.sock`)); await c.init(); clients.push(c); }
    const [a, b] = clients;
    const task = async (c, title) => (await c.tool('task_create', { title, detail: '', depends_on: [] })).task_id;
    await task(a, 'Write the release notes for 0.4');
    await task(a, 'Pass the clock into is_expired so the expiry test is deterministic');
    const doing = await task(b, 'Port the parser to the new token API');
    await b.tool('task_claim', { task_id: doing });
    await b.tool('lease_acquire', { paths: ['src/parser.rs'], ttl_s: 600, reason: 'porting' });
    const done = await task(a, 'Fix the flaky login test');
    await a.tool('task_claim', { task_id: done });
    await a.tool('task_complete', { task_id: done, summary: 'fixed' });
    await a.tool('send_message', { to: helper, body: 'I am taking src/auth.rs. Please stay out of it until I push.' });
    await b.tool('send_message', { to: shell, body: 'Fine. The parser port touches src/parser.rs only; I will run the whole suite before I mark it done.' });
    const panes = (p1, p2) => page.evaluate((v) => localStorage.setItem('agora.workspace.v1', v), layout(split([0.55, 0.45], p1, p2), 'p2'));
    // On a desktop the board and Team chat are panes beside Home; on a phone each is a screen of its own, reached from More in the
    // bottom bar (finding 15), shot here at its own address.
    await each('board', async (width) => {
      await panes(pane('p1', 'fleet'), pane('p2', 'tasks'));
      await page.goto(`${origin}/#/${width === 'phone' ? 'board' : ''}`); await page.reload();
      await page.getByRole('button', { name: /^Open board/ }).locator('visible=true').first().click();
      await page.getByRole('dialog').getByText('Port the parser to the new token API').waitFor();
    });
    await each('team-chat', async (width) => {
      await panes(pane('p1', 'fleet'), pane('p2', 'messages'));
      await page.goto(`${origin}/#/${width === 'phone' ? 'messages' : ''}`); await page.reload();
      await page.getByText('I am taking src/auth.rs', { exact: false }).first().waitFor({ timeout: 15000 });
    });
    // A machine asking to join (ADR-0065): the owner allows new machines from My machines, `agora join` runs on the new machine (here a
    // second home on this one), and the card with its number shows on My machines. The join is stopped once shot; nothing joins.
    if (want('pairing')) {
      await page.goto(`${origin}/#/machines`);
      await page.getByRole('button', { name: 'Add a machine', exact: true }).first().click();
      await page.getByRole('button', { name: 'Allow a new machine to join', exact: true }).click();
      await page.waitForTimeout(500);
      await page.keyboard.press('Escape');
      const joinHome = mkdtempSync(join(tmpdir(), 'vpj-'));
      const joining = spawn(bin, ['join', `localhost:${process.env.AGORA_TEST_PORT}`, '--id', 'desk'], { env: { ...env, AGORA_HOME: joinHome, HOSTNAME: 'desk' }, stdio: 'ignore' });
      try {
        await each('pairing', async () => { await page.goto(`${origin}/#/machines`); await page.getByText('desk').first().waitFor({ timeout: 30000 }); });
      } finally { joining.kill(); rmSync(joinHome, { recursive: true, force: true }); }
    }
    if (errors.length) { failed++; console.log(`FAIL live page errors: ${errors.join(' | ')}`); }
  } finally {
    for (const c of clients) c.close();
    await stack.close();
  }
}

const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox', '--font-render-hinting=none'] });
try {
  if (part !== 'live') await staticPart(browser);
} finally { await browser.close(); }
if (part !== 'static') await livePart();
rmSync(scratch, { recursive: true, force: true });
if (wantAudit) writeFileSync(join(out, 'audit.json'), `${JSON.stringify(audits, null, 1)}\n`);
console.log(failed ? `${failed} failed` : 'all shots taken');
process.exit(failed ? 1 : 0);
