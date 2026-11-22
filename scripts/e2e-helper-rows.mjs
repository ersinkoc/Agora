// Real-DOM check for the Agents list's rows (#945, RM-21): a helper row once kept only "." of its name at 125 % because the name shared a
// truncating box with "started by …" and the badges never gave way. The list is drawn at its narrowest (200 px), default (272 px) and widest
// (480 px) width, at 100 % and 125 % and in Large text, with a parent, two helpers and another agent that acts freely, all in one shared
// folder. Each row must show its whole name or at least 64 px of it (about eight characters).
//
//   cd apps/web && npm ci && npm run gen
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-helper-rows.mjs
//   AGORA_SCREENSHOTS=docs/ux/screens PW_DIR=… node scripts/e2e-helper-rows.mjs   # also writes agents-helpers-{100,125}-{light,dark}.png
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
import '/src/design/app.css';
import { initTheme } from '/src/themes/store.ts';
initTheme();
const base = { machine: 'WHITE', machineId: 'white', workspace: 'proj', project: 'proj', state: 'working', preview: '', accent: 'var(--agent-amber)', shared: true };
const fleet = [
  { ...base, id: 'white/a1', name: 'rm-acp · amber' },
  { ...base, id: 'white/a2', name: 'rm-helper · teal', parentId: 'white/a1', parentName: 'rm-acp · amber' },
  { ...base, id: 'white/a3', name: 'rm-helper · sky', parentId: 'white/a1', parentName: 'rm-acp · amber', skipsPermissions: true },
  { ...base, id: 'white/a4', name: 'rm-helper · rose', parentId: 'white/a2', parentName: 'rm-helper · teal', filesChanged: 3 },
  { ...base, id: 'white/a5', name: 'rm-probe · slate', skipsPermissions: true },
];
const machines = [{ id: 'white', name: 'WHITE', os: 'windows', online: true, agentsRunning: 5, lastSeenMs: Date.now() }];
const noop = () => {};
const width = Number(new URLSearchParams(location.search).get('w') ?? 272);
createRoot(document.getElementById('root')).render(h('div', { id: 'list', style: { width: width + 'px', height: '360px', display: 'grid', gridTemplateColumns: 'minmax(0,1fr)' } },
  h(Explorer, { by: 'machine', onBy: noop, spread: new Map(), fleet, machines, open: [], current: 'white/a3', projects: [], project: '', onProject: noop, onToggle: noop, onCollapse: noop, onDismiss: noop })));
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
try {
  for (const [scale, width, choice] of [[1, 200], [1, 272], [1, 480], [1.25, 200], [1.25, 272], [1.25, 480], [1.5, 272], [1, 272, 'large-text'], [1.25, 272, 'large-text']]) {
    const label = `${Math.round(scale * 100)} % ${width} px${choice ? ` ${choice}` : ''}`;
    // 125 % in Chrome on a 1920×1080 screen: 1536×864 CSS px drawn at a device scale of 1.25.
    const ctx = await browser.newContext({ viewport: { width: Math.round(1920 / scale), height: Math.round(1080 / scale) }, deviceScaleFactor: scale, colorScheme: 'dark' });
    const page = await ctx.newPage();
    if (choice) await page.addInitScript((c) => { localStorage.setItem('agora.theme', 'dark'); localStorage.setItem('agora.theme.choice.v1', JSON.stringify({ dark: c, light: 'light' })); }, choice);
    await page.goto(`${base}?w=${width}`);
    await page.waitForFunction(() => window.__ready === true && document.querySelectorAll('[data-agent-row]').length === 5);
    const rows = await page.$$eval('[data-agent-row]', (els) => els.map((row) => {
      const name = row.querySelector('[data-agent-name]') ?? row.querySelector('.truncate');
      return { text: name.textContent, shown: name.clientWidth, full: name.scrollWidth, row: row.clientWidth, over: row.scrollWidth > row.clientWidth + 1 };
    }));
    for (const r of rows) {
      const ok = r.shown + 1 >= Math.min(r.full, MIN_SHOWN);
      if (!ok) failed++;
      console.log(`${ok ? 'ok  ' : 'FAIL'} ${label} ${r.text}: ${r.shown}/${r.full} px of the name shown in a ${r.row} px row`);
    }
    if (shots && !choice && width === 272 && (scale === 1 || scale === 1.25)) {
      for (const theme of ['light', 'dark']) {
        await page.evaluate((t) => { document.documentElement.dataset.theme = t; window.dispatchEvent(new Event('agora-theme')); }, theme);
        await page.locator('#list').screenshot({ path: join(shots, `agents-helpers-${Math.round(scale * 100)}-${theme}.png`), animations: 'disabled' });
      }
    }
    await ctx.close();
  }
} finally {
  await browser.close();
  await server.close();
}
process.exit(failed ? 1 : 0);
