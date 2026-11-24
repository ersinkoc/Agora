// Real-DOM check for the fleet console's attention rows (a decision is never taken on cut text): the longest request that is answered in
// its row (apps/web/src/net/fleet.ts, INLINE_MAX) is drawn at 340, 420 and 820 px and in Large text, and none of its text may be clipped
// (scrollHeight <= clientHeight and scrollWidth <= clientWidth). One more than the limit must not have Allow at all.
//
//   cd apps/web && npm ci && npm run gen
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-fleet-rows.mjs
//
// Runs the web app's own Vite dev server on a free port with a page of its own made in memory (no files are added to apps/web).
import { createRequire } from 'node:module';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const web = join(root, 'apps/web');
const requireWeb = createRequire(join(web, 'package.json'));
const { createServer } = await import(requireWeb.resolve('vite'));
const react = (await import(requireWeb.resolve('@vitejs/plugin-react'))).default;
const tailwind = (await import(requireWeb.resolve('@tailwindcss/vite'))).default;
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;

const MAX = 120;
const ACTION = 'Edit src/auth.rs';
const longest = `${'w'.repeat(MAX - ACTION.length - 3)}`; // widest ordinary text, in words below
const words = Array.from({ length: 40 }, () => 'abcd').join(' ').slice(0, MAX - ACTION.length - 3);
const cases = [
  { id: 'words', action: ACTION, detail: words },
  { id: 'unbroken', action: ACTION, detail: longest },
  { id: 'wide', action: 'W'.repeat(MAX), detail: '' },
  { id: 'over', action: ACTION, detail: 'x'.repeat(MAX - ACTION.length - 3 + 1), over: true },
];

const virtual = `
import { createElement as h } from 'react';
import { createRoot } from 'react-dom/client';
import { AttentionQueue } from '/src/components/FleetConsole.tsx';
import '/src/design/app.css';
import { initTheme } from '/src/themes/store.ts';
initTheme();
const cases = ${JSON.stringify(cases)};
const items = cases.map((c, i) => ({ kind: 'approval', key: 'r' + i, approval: { id: 'r' + i, node: 'n', localId: String(i), agentId: 'a' + i, agentName: 'kind' + i + ' · amber', action: c.action, detail: c.detail, expiresAtMs: 0 },
  agent: { id: 'a' + i, name: 'kind' + i + ' · amber', accent: '#fff', machine: 'laptop', workspace: 'w', project: 'login-fix', state: 'needs-you', preview: '' } }));
createRoot(document.getElementById('root')).render(h('div', { className: 'fleet-console' }, h(AttentionQueue, { items, machines: [], now: Date.now(), onOpen() {}, onResolve() {} })));
window.__ready = true;
`;
const server = await createServer({
  root: web, configFile: false, logLevel: 'error', server: { port: 0, host: '127.0.0.1' },
  plugins: [react(), tailwind(), {
    name: 'fleet-rows-page',
    resolveId: (id) => (id === 'virtual:fleet-rows' ? '\0virtual:fleet-rows' : undefined),
    load: (id) => (id === '\0virtual:fleet-rows' ? virtual : undefined),
    configureServer(s) {
      s.middlewares.use('/__fleet-rows.html', async (req, res) => {
        const html = await s.transformIndexHtml('/__fleet-rows.html', '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">import "virtual:fleet-rows";</script></body></html>');
        res.setHeader('content-type', 'text/html'); res.end(html);
      });
    },
  }],
});
await server.listen();
const base = `http://127.0.0.1:${server.httpServer.address().port}/__fleet-rows.html`;
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
let failed = 0;
for (const [name, width, choice] of [['340', 340], ['420', 420], ['820', 820], ['large-text 340', 340, 'large-text'], ['large-text 820', 820, 'large-text']]) {
  const ctx = await browser.newContext({ viewport: { width, height: 900 }, colorScheme: 'dark' });
  const page = await ctx.newPage();
  if (choice) await page.addInitScript((c) => { localStorage.setItem('agora.theme', 'dark'); localStorage.setItem('agora.theme.choice.v1', JSON.stringify({ dark: c, light: 'light' })); }, choice);
  await page.goto(base);
  await page.waitForFunction(() => window.__ready === true);
  const rows = await page.$$eval('li.fleet-ask', (lis) => lis.map((li) => {
    const what = li.querySelector('.fleet-ask__what');
    return { allow: [...li.querySelectorAll('button')].some((b) => b.textContent === 'Allow'), h: what.scrollHeight, ch: what.clientHeight, w: what.scrollWidth, cw: what.clientWidth, text: what.textContent.length };
  }));
  rows.forEach((r, i) => {
    const c = cases[i];
    const clipped = r.h > r.ch + 1 || r.w > r.cw + 1;
    const ok = c.over ? !r.allow : r.allow && !clipped;
    if (!ok) { failed++; console.log(`FAIL ${name} ${c.id}`, JSON.stringify(r)); } else console.log(`ok   ${name} ${c.id} ${r.text} chars, ${r.h}/${r.ch} px, Allow ${r.allow ? 'yes' : 'no'}`);
  });
  await ctx.close();
}
await browser.close();
await server.close();
process.exit(failed ? 1 : 0);
