// Real-DOM check for the Agents list's rows (#945, RM-21): a helper row once kept only "." of its name at 125 % because the name shared a
// truncating box with "started by …" and the badges never gave way. The list is drawn at its narrowest (200 px), default (272 px) and widest
// (480 px) width, at 100 % and 125 % and in Large text, with a parent, two helpers and another agent that acts without asking, all in one
// shared folder, plus an agent that only acts without asking and one that is only near its plan limit. Each row must show its whole name or
// at least 64 px of it (about eight characters); every acts-without-asking sign (a red bolt, no words) must sit whole inside its row; and the
// limit chip must never read like it (Codex review of #989: a lone warning sign cannot tell skipped approvals from quota pressure), so the
// two must differ in glyph and colour and the chip keeps its percentage. The sign's tooltip must open on hover and on the row's keyboard
// focus and lie wholly inside the window. A second page draws an agent's session bar in a mode that skips approvals and checks the same.
//
//   cd apps/web && npm ci && npm run gen
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-helper-rows.mjs
//   AGORA_SCREENSHOTS=docs/ux/screens PW_DIR=… node scripts/e2e-helper-rows.mjs   # also writes agents-helpers-{100,125,125-narrow}-{light,dark}.png,
//                                                                                 # agents-no-ask-tip-{light,dark}.png and session-no-ask-tip-{light,dark}.png
//
// Runs the web app's own Vite dev server on a free port with a page of its own made in memory (no files are added to apps/web).
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const web = join(root, 'apps/web');
const requireWeb = createRequire(join(web, 'package.json'));
const { createServer } = await import(requireWeb.resolve('vite'));
const react = (await import(requireWeb.resolve('@vitejs/plugin-react'))).default;
const tailwind = (await import(requireWeb.resolve('@tailwindcss/vite'))).default;
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const shots = process.env.AGORA_SCREENSHOTS ? resolve(root, process.env.AGORA_SCREENSHOTS) : '';
const MIN_SHOWN = 64;

const virtual = `
import { createElement as h } from 'react';
import { createRoot } from 'react-dom/client';
import { Explorer } from '/src/components/Explorer.tsx';
import { QuotaProvider } from '/src/components/Quota.tsx';
import { SessionBar } from '/src/components/SessionBar.tsx';
import { setLanguage } from '/src/i18n.ts';
import '/src/design/app.css';
import { initTheme } from '/src/themes/store.ts';
initTheme();
setLanguage('en');
const base = { machine: 'WHITE', machineId: 'white', workspace: 'proj', project: 'proj', state: 'working', preview: '', accent: 'var(--agent-amber)', shared: true };
const fleet = [
  { ...base, id: 'white/a1', name: 'rm-acp · amber' },
  { ...base, id: 'white/a2', name: 'rm-helper · teal', parentId: 'white/a1', parentName: 'rm-acp · amber' },
  { ...base, id: 'white/a3', name: 'rm-helper · sky', parentId: 'white/a1', parentName: 'rm-acp · amber', skipsPermissions: true },
  { ...base, id: 'white/a4', name: 'rm-helper · rose', parentId: 'white/a2', parentName: 'rm-helper · teal', filesChanged: 3 },
  { ...base, id: 'white/a5', name: 'rm-probe · slate', skipsPermissions: true },
  { ...base, id: 'white/a6', name: 'claude · plum', workspace: 'other', project: 'other', shared: false, skipsPermissions: true },
  { ...base, id: 'white/a7', name: 'codex · lime', workspace: 'other', project: 'other', shared: false },
];
const now = Date.now();
const obs = [{ machineId: 'white', tool: 'codex', window: 'five_hour', usedPercent: 85, resetsAtMs: now + 3600000, observedAtMs: now - 60000 }];
const machines = [{ id: 'white', name: 'WHITE', os: 'windows', online: true, agentsRunning: 5, lastSeenMs: Date.now() }];
const noop = () => {};
const params = new URLSearchParams(location.search);
const width = Number(params.get('w') ?? 272);
// An agent's session bar (the row under its prompt) after a switch to a mode that skips approvals.
const opt = (id, name, skipsPermissions = false) => ({ id, name, description: '', skipsPermissions });
const session = { modes: [opt('default', 'Default'), opt('bypassPermissions', 'Bypass permissions', true)], models: [opt('sonnet', 'Sonnet')], currentMode: 'bypassPermissions', currentModel: 'sonnet', contextTokens: 48000, contextWindow: 200000 };
createRoot(document.getElementById('root')).render(params.get('page') === 'session'
  ? h('div', { id: 'bar', className: 'bg-surface-0', style: { width: width + 'px', padding: '12px 12px 72px' } },
    h('div', { className: 'flex flex-wrap items-center gap-x-3 gap-y-1.5' }, h(SessionBar, { part: 'controls', agentKey: 'white/a1', session, choose: async () => true })))
  : h('div', { id: 'list', style: { width: width + 'px', height: '480px', display: 'grid', gridTemplateColumns: 'minmax(0,1fr)' } },
    h(QuotaProvider, { obs, machines: new Map([['white', 'WHITE']]) },
      h(Explorer, { by: 'machine', onBy: noop, spread: new Map(), fleet, machines, open: [], current: 'white/a3', projects: [], project: '', onProject: noop, onToggle: () => { window.__toggled = (window.__toggled ?? 0) + 1; }, onCollapse: noop, onDismiss: noop }))));
window.__ready = true;
`;
const server = await createServer({
  root: web, configFile: false, logLevel: 'error', server: { port: 0, host: '127.0.0.1' },
  plugins: [react(), tailwind(), {
    name: 'helper-rows-page',
    resolveId: (id) => (id === 'virtual:helper-rows' ? '\0virtual:helper-rows' : undefined),
    load: (id) => (id === '\0virtual:helper-rows' ? virtual : undefined),
    configureServer(s) {
      s.middlewares.use('/__helper-rows.html', async (req, res) => {
        const html = await s.transformIndexHtml('/__helper-rows.html', '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">import "virtual:helper-rows";</script></body></html>');
        res.setHeader('content-type', 'text/html'); res.end(html);
      });
    },
  }],
});
await server.listen();
const base = `http://127.0.0.1:${server.httpServer.address().port}/__helper-rows.html`;
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
let failed = 0;

// The open tooltip of the acts-without-asking sign: present, saying what the sign's name says, and wholly inside the window.
async function tipCheck(page, label) {
  await page.waitForTimeout(50);
  const r = await page.evaluate(() => {
    const tip = document.querySelector('[data-no-ask-tip]');
    if (!tip) return { open: false };
    const b = tip.getBoundingClientRect();
    const owner = tip.closest('[data-no-ask]') ?? document.querySelector('[data-no-ask]');
    return { open: true, text: tip.textContent, visible: getComputedStyle(tip).visibility !== 'hidden', inside: b.left >= 0 && b.top >= 0 && b.right <= document.documentElement.clientWidth && b.bottom <= window.innerHeight, w: Math.round(b.width), name: owner?.getAttribute('aria-label') };
  });
  const good = r.open && r.visible && r.inside && r.text?.startsWith('Runs without asking: ');
  console.log(`${good ? 'ok  ' : 'FAIL'} ${label}: tooltip ${JSON.stringify(r)}`);
  return good;
}

// A screenshot clip around `sel` and the open tooltip together.
async function around(page, sel) {
  return page.evaluate((s) => {
    const boxes = [document.querySelector(s), document.querySelector('[data-no-ask-tip]')].filter(Boolean).map((e) => e.getBoundingClientRect());
    const x = Math.max(0, Math.min(...boxes.map((b) => b.left)) - 4), y = Math.max(0, Math.min(...boxes.map((b) => b.top)) - 4);
    return { x, y, width: Math.max(...boxes.map((b) => b.right)) + 4 - x, height: Math.max(...boxes.map((b) => b.bottom)) + 4 - y };
  }, sel);
}
try {
  for (const [scale, width, choice] of [[1, 200], [1, 272], [1, 480], [1.25, 200], [1.25, 272], [1.25, 480], [1.5, 272], [1, 272, 'large-text'], [1.25, 272, 'large-text']]) {
    const label = `${Math.round(scale * 100)} % ${width} px${choice ? ` ${choice}` : ''}`;
    // 125 % in Chrome on a 1920×1080 screen: 1536×864 CSS px drawn at a device scale of 1.25.
    const ctx = await browser.newContext({ viewport: { width: Math.round(1920 / scale), height: Math.round(1080 / scale) }, deviceScaleFactor: scale, colorScheme: 'dark' });
    const page = await ctx.newPage();
    if (choice) await page.addInitScript((c) => { localStorage.setItem('agora.theme', 'dark'); localStorage.setItem('agora.theme.choice.v1', JSON.stringify({ dark: c, light: 'light' })); }, choice);
    await page.goto(`${base}?w=${width}`);
    await page.waitForFunction(() => window.__ready === true && document.querySelectorAll('[data-agent-row]').length === 7);
    const rows = await page.$$eval('[data-agent-row]', (els) => els.map((row) => {
      const name = row.querySelector('[data-agent-name]') ?? row.querySelector('.truncate');
      const box = row.getBoundingClientRect();
      const badge = (sel) => {
        const el = row.querySelector(sel);
        if (!el) return undefined;
        const b = el.getBoundingClientRect();
        return { text: el.innerText.trim(), inside: b.left >= box.left - 0.5 && b.right <= box.right + 0.5 && b.bottom <= box.bottom + 0.5, cut: el.scrollWidth > el.clientWidth + 1, w: Math.round(b.width),
          glyph: el.querySelector('svg path')?.getAttribute('d') ?? '', color: getComputedStyle(el).color, name: el.getAttribute('aria-label') ?? '' };
      };
      return { text: name.textContent, shown: name.clientWidth, full: name.scrollWidth, row: row.clientWidth, free: badge('[data-no-ask]'), limit: badge('[data-level]') };
    }));
    for (const r of rows) {
      const ok = r.shown + 1 >= Math.min(r.full, MIN_SHOWN);
      if (!ok) failed++;
      console.log(`${ok ? 'ok  ' : 'FAIL'} ${label} ${r.text}: ${r.shown}/${r.full} px of the name shown in a ${r.row} px row`);
      if (r.free) {
        const good = r.free.text === '' && r.free.name.startsWith('Runs without asking: ') && r.free.inside && !r.free.cut && r.free.w <= 32; // one sign, no words: about 1.5rem, 30 px in Large text
        if (!good) failed++;
        console.log(`${good ? 'ok  ' : 'FAIL'} ${label} ${r.text}: acts-without-asking sign ${JSON.stringify({ ...r.free, glyph: undefined })}`);
      }
      if (r.limit) {
        const good = /^(Limit )?85 %$/.test(r.limit.text) && r.limit.inside && !r.limit.cut;
        if (!good) failed++;
        console.log(`${good ? 'ok  ' : 'FAIL'} ${label} ${r.text}: limit chip ${JSON.stringify(r.limit)}`);
      }
    }
    // The agent that only acts freely and the one only near its limit, side by side at this width: they must not look alike.
    const free = rows.find((r) => r.text === 'claude · plum')?.free;
    const limit = rows.find((r) => r.text === 'codex · lime')?.limit;
    const apart = !!free && !!limit && free.glyph !== '' && limit.glyph !== '' && free.glyph !== limit.glyph && free.color !== limit.color && free.text === '' && /85 %$/.test(limit.text);
    if (!apart) failed++;
    console.log(`${apart ? 'ok  ' : 'FAIL'} ${label} the sign (${free?.color}, no words) and the limit chip (${limit?.color}, "${limit?.text}") differ in glyph and colour`);
    // Keyboard: the row that holds the sign gets focus the way Tab gives it; the tooltip opens and lies wholly inside the window.
    await page.focus('[data-agent-row="white/a6"]');
    if (!(await tipCheck(page, `${label} row focus`))) failed++;
    await page.locator('[data-agent-row="white/a6"]').evaluate((el) => el.blur());
    await page.hover('[data-agent-row="white/a6"] [data-no-ask]');
    if (!(await tipCheck(page, `${label} hover`))) failed++;
    await page.mouse.move(0, 0);
    if (shots && !choice && (width === 272 ? scale === 1 || scale === 1.25 : width === 200 && scale === 1.25)) {
      for (const theme of ['light', 'dark']) {
        await page.evaluate((t) => { document.documentElement.dataset.theme = t; window.dispatchEvent(new Event('agora-theme')); }, theme);
        await page.locator('#list').screenshot({ path: join(shots, `agents-helpers-${Math.round(scale * 100)}${width === 200 ? '-narrow' : ''}-${theme}.png`), animations: 'disabled' });
      }
      if (scale === 1 && width === 272) {
        for (const theme of ['light', 'dark']) {
          await page.evaluate((t) => { document.documentElement.dataset.theme = t; window.dispatchEvent(new Event('agora-theme')); }, theme);
          await page.focus('[data-agent-row="white/a6"]');
          await page.screenshot({ path: join(shots, `agents-no-ask-tip-${theme}.png`), clip: await around(page, '#list'), animations: 'disabled' });
          await page.locator('[data-agent-row="white/a6"]').evaluate((el) => el.blur());
        }
      }
    }
    await ctx.close();
  }
  // The session bar: switched to a mode that skips approvals, so the sign says it was switched and names the mode.
  for (const [scale, width] of [[1, 640], [1.25, 360]]) {
    const label = `session bar ${Math.round(scale * 100)} % ${width} px`;
    const ctx = await browser.newContext({ viewport: { width: Math.round(1920 / scale), height: Math.round(1080 / scale) }, deviceScaleFactor: scale, colorScheme: 'dark' });
    const page = await ctx.newPage();
    await page.goto(`${base}?page=session&w=${width}`);
    await page.waitForFunction(() => window.__ready === true && !!document.querySelector('[data-no-ask]'));
    const sign = await page.$eval('[data-no-ask]', (el) => ({ kind: el.dataset.noAsk, text: el.innerText.trim(), name: el.getAttribute('aria-label'), focusable: el.tabIndex === 0 }));
    const good = sign.kind === 'session' && sign.text === '' && sign.focusable && sign.name === 'Runs without asking: Acts without asking you first, until it ends or is switched back. Because of: Mode: Bypass permissions';
    if (!good) failed++;
    console.log(`${good ? 'ok  ' : 'FAIL'} ${label}: ${JSON.stringify(sign)}`);
    await page.focus('[data-no-ask]');
    if (!(await tipCheck(page, `${label} focus`))) failed++;
    await page.keyboard.press('Escape');
    const closed = (await page.$('[data-no-ask-tip]')) === null;
    if (!closed) failed++;
    console.log(`${closed ? 'ok  ' : 'FAIL'} ${label}: Esc closes the tooltip`);
    // A touch screen: a tap on the sign opens its tooltip, a tap elsewhere closes it.
    const touch = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, hasTouch: true, isMobile: true, colorScheme: 'dark' });
    const phone = await touch.newPage();
    await phone.goto(`${base}?page=session&w=370`);
    await phone.waitForFunction(() => window.__ready === true && !!document.querySelector('[data-no-ask]'));
    await phone.tap('[data-no-ask]');
    if (!(await tipCheck(phone, `${label} phone tap`))) failed++;
    await phone.tap('#bar', { position: { x: 4, y: 4 } });
    await phone.waitForTimeout(50);
    const gone = (await phone.$('[data-no-ask-tip]')) === null;
    if (!gone) failed++;
    console.log(`${gone ? 'ok  ' : 'FAIL'} ${label} phone: a tap elsewhere closes the tooltip`);
    await touch.close();
    if (shots && scale === 1) {
      for (const theme of ['light', 'dark']) {
        await page.evaluate((t) => { document.documentElement.dataset.theme = t; window.dispatchEvent(new Event('agora-theme')); }, theme);
        await page.locator('[data-no-ask]').evaluate((el) => el.blur());
        await page.focus('[data-no-ask]');
        await page.screenshot({ path: join(shots, `session-no-ask-tip-${theme}.png`), clip: await around(page, '#bar'), animations: 'disabled' });
      }
    }
    await ctx.close();
  }
  // Codex review of #1004. A tap on the bolt inside an Agents-list row shows its tooltip and does not open the row (a tap on the name
  // still does); and the pointer can move from the bolt onto the tooltip and rest there without it closing (WCAG 1.4.13).
  {
    const touch = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, hasTouch: true, isMobile: true, colorScheme: 'dark' });
    const phone = await touch.newPage();
    await phone.goto(`${base}?w=370`);
    await phone.waitForFunction(() => window.__ready === true && document.querySelectorAll('[data-agent-row]').length === 7);
    await phone.tap('[data-agent-row="white/a6"] [data-no-ask]');
    if (!(await tipCheck(phone, 'Agents list phone tap on the bolt'))) failed++;
    const kept = (await phone.evaluate(() => window.__toggled ?? 0)) === 0;
    if (!kept) failed++;
    console.log(`${kept ? 'ok  ' : 'FAIL'} Agents list phone: a tap on the bolt does not open the row`);
    await phone.tap('[data-agent-row="white/a6"] [data-agent-name]');
    const opened = (await phone.evaluate(() => window.__toggled ?? 0)) === 1 && (await phone.$('[data-no-ask-tip]')) === null;
    if (!opened) failed++;
    console.log(`${opened ? 'ok  ' : 'FAIL'} Agents list phone: a tap on the name opens the row and closes the tooltip`);
    await touch.close();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: 'dark' });
    const page = await ctx.newPage();
    await page.goto(`${base}?w=272`);
    await page.waitForFunction(() => window.__ready === true && document.querySelectorAll('[data-agent-row]').length === 7);
    await page.hover('[data-agent-row="white/a6"] [data-no-ask]');
    await page.waitForSelector('[data-no-ask-tip]');
    const b = await page.$eval('[data-no-ask-tip]', (el) => { const r = el.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; });
    await page.mouse.move(b.x, b.y, { steps: 8 });
    await page.waitForTimeout(800);
    if (!(await tipCheck(page, 'Agents list: the pointer rests on the tooltip'))) failed++;
    await page.mouse.move(1200, 700, { steps: 4 });
    await page.waitForTimeout(800);
    const shut = (await page.$('[data-no-ask-tip]')) === null;
    if (!shut) failed++;
    console.log(`${shut ? 'ok  ' : 'FAIL'} Agents list: leaving the tooltip closes it`);
    await ctx.close();
  }
} finally {
  await browser.close();
  await server.close();
}
process.exit(failed ? 1 : 0);
