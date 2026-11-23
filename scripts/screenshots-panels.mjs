// The side panel's lists (docs/ux/main-window.md, "The side panel's lists"): renders the real Inspector with long team chat messages, a
// board and usage through the web app's own Vite (no hub), checks in Chromium that a long message is cut to two lines, that HTML in a message
// is text, that each row's line fits, and that Enter opens the whole text and Esc returns to the row with the panel still open; then writes
// docs/ux/screens/panels-redesign-*, dark and light, desktop and a 390 px drawer. Exit 1 on a failed check.
//   cd apps/web && npm ci
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots-panels.mjs [--check-only]
import { createRequire } from 'node:module';
import { join } from 'node:path';
const web = new URL('../apps/web', import.meta.url).pathname;
const out = new URL('../docs/ux/screens', import.meta.url).pathname;
const checkOnly = process.argv.includes('--check-only');
const requireWeb = createRequire(join(web, 'package.json'));
const { createServer } = await import(requireWeb.resolve('vite'));
const react = (await import(requireWeb.resolve('@vitejs/plugin-react'))).default;
const tailwind = (await import(requireWeb.resolve('@tailwindcss/vite'))).default;
const { chromium } = createRequire(join(process.env.PW_DIR, 'x.js'))('playwright-core');
const long = 'Fine. While you are there: the expiry check compares against SystemTime::now() inside is_expired, which makes it impossible to test deterministically, so I would pass the clock in. Also the token lifetime is hardcoded as 3600 seconds in issue(), but the spec we agreed on in the planning thread says fifteen minutes for access tokens and seven days for refresh tokens, and the refresh path in src/refresh.rs does not exist yet. I can write the refresh path once you push, and I will add the tests for the expired-token and wrong-signature cases, plus a regression test for the clock skew bug from last week (tokens issued on the build server were rejected by the laptop because its clock ran 40 seconds behind).';
const prompt = 'Write unit tests for src/auth.rs.\n\nCover these cases:\n1. A token exactly at its expiry is still valid.\n2. A token one second past its expiry is refused.\n3. A token whose signature was made with another key is refused, and the error says which check failed.\n4. A token issued with a clock 40 seconds ahead is accepted within the skew window.\n\nDo not change anything under src/ except tests; if a case cannot be tested without changing the code, write it down in the task board instead of changing the code. Run the tests with cargo test -p login auth:: and report the output.';
const virtual = `
import { createElement as h, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Details, Inspector } from '/src/components/Inspector.tsx';
import { t as tr } from '/src/i18n.ts';
import '/src/design/app.css';
import { initTheme } from '/src/themes/store.ts';
import { setLanguage } from '/src/i18n.ts';
initTheme();
const q = new URLSearchParams(location.search);
setLanguage(q.get('lang') || 'en');
const now = Date.now();
const fleet = [
  { id: 'n/a1', name: 'codex · teal', kind: 'codex', accent: 'var(--agent-teal)', machine: 'blackview', workspace: 'agora', project: 'agora', state: 'working', preview: '', tokens: 182400, startedMs: now - 3_600_000, session: { modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 62000, contextWindow: 200000 } },
  { id: 'n/a2', name: 'claude · plum', kind: 'claude', accent: 'var(--agent-violet)', machine: 'blackview', workspace: 'agora', project: 'agora', state: 'idle', preview: '', tokens: 96100, startedMs: now - 2_400_000 },
  { id: 'n/a3', name: 'shell · violet', kind: 'shell', accent: 'var(--agent-violet)', machine: 'blackview', workspace: 'agora', project: 'agora', state: 'idle', preview: '' },
  { id: 'm/a4', name: 'codex · mint', kind: 'codex', accent: 'var(--agent-mint)', machine: 'mac', workspace: 'macproj', project: 'macproj', state: 'working', preview: '', tokens: 41000, startedMs: now - 900_000 },
];
const messages = [
  { from: 'codex · teal', fromId: 'n/a1', to: 'claude · plum', body: ${JSON.stringify(long)}, atMs: now - 60000, crossMachine: false },
  { from: 'shell · violet', fromId: 'n/a3', to: 'codex · mint', toMachine: 'mac', body: ${JSON.stringify(prompt)}, atMs: now - 95000, crossMachine: true },
  { from: 'claude · plum', fromId: 'n/a2', to: '#general', body: 'Heads up: the test suite takes about four minutes now; run only the auth tests while iterating (cargo test -p login auth::).', atMs: now - 120000, crossMachine: false },
  { from: 'claude · plum', fromId: 'n/a2', to: 'codex · teal', body: 'I am taking src/auth.rs. Please stay out of it until I am done.', atMs: now - 180000, crossMachine: false },
  { from: 'win/gemini · rose', fromId: 'win/a9', to: 'codex · teal', toMachine: 'blackview', body: 'Build on win failed: error[E0433]: failed to resolve: use of undeclared crate or module \`windows_sys\` --> crates/agora-platform/src/windows.rs:12:5 <b>not bold</b>', atMs: now - 240000, crossMachine: true, via: 'win' },
];
const boards = [{ projectId: '551ed26b65d1f0c4a9e2b7d3c8f1a6e5b4d3c2a1', project: 'agora', folders: [{ nodeId: 'n', machine: 'blackview', path: '/home/dev/code/agora' }, { nodeId: 'm', machine: 'mac', path: '/Users/dev/agora' }],
  tasks: [
    { id: 'p/1', title: 'Write unit tests for src/auth.rs: a token exactly at its expiry, one second past it, a token signed with another key, and a clock 40 seconds ahead within the skew window', status: 'claimed', owner: 'codex · teal' },
    { id: 'p/2', title: 'Pass the clock into is_expired', status: 'open', owner: '' },
    { id: 'p/3', title: 'Fifteen-minute access tokens', status: 'done', owner: 'claude · plum' },
  ], leases: [{ holder: 'claude · plum', path: 'src/auth.rs' }] }];
const tab = q.get('tab');
const empty = q.get('empty') === '1';
// The layout's Usage panel (the rail offers Usage only once there is something to measure): the same title row and Details.
function Panel() {
  return h('aside', { className: 'inspector' }, h('section', { className: 'inspector__panel' },
    h('div', { className: 'flex h-9 flex-none items-center justify-between gap-2 border-b border-line ps-3 pe-1' }, h('h2', { className: 'panel-title' }, tr('ide-tab-usage'))),
    h('div', { className: 'min-h-0 flex-1 overflow-y-auto p-3' }, h(Details, { tab, fleet: [], now, attention: [], approvals: [], changed: [], boards: [], messages: [], onOpen() {}, onResolve() {} }))));
}
function App() {
  const [open, setOpen] = useState(true);
  const [t, setT] = useState(tab);
  return h(Inspector, { fleet: empty ? [] : fleet, now, attention: [], approvals: [], changed: [], boards: empty ? [] : boards, messages: empty ? [] : messages, projects: [], onOpen() {}, onResolve() {},
    open, onOpenChange: setOpen, tab: t, onTab: setT });
}
const phone = innerWidth < 1024;
document.body.style.margin = '0';
createRoot(document.getElementById('root')).render(h('div', { style: { display: 'grid', height: '100vh', gridTemplateColumns: phone ? '1fr 2.75rem' : '1fr calc(336px + 2.75rem)' } },
  h('div', { className: 'bg-surface-0' }), h(q.get('details') ? Panel : App)));
window.__ready = true;
`;
const server = await createServer({ root: web, configFile: false, logLevel: 'error', server: { port: 0, host: '127.0.0.1', fs: { allow: [join(web, '../..')] } },
  plugins: [react(), tailwind(), { name: 'side', resolveId: (id) => (id === 'virtual:side' ? '\0virtual:side' : undefined), load: (id) => (id === '\0virtual:side' ? virtual : undefined),
    configureServer(s) { s.middlewares.use('/__side.html', async (req, res) => { const html = await s.transformIndexHtml('/__side.html', '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"></head><body><div id="root"></div><script type="module">import "virtual:side";</script></body></html>'); res.setHeader('content-type', 'text/html'); res.end(html); }); } }] });
await server.listen();
const base = `http://127.0.0.1:${server.httpServer.address().port}/__side.html`;
const browser = await chromium.launch({ executablePath: process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`, args: ['--no-sandbox', '--font-render-hinting=none'] });
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const shots = [
  ['team-chat', 'messages', false], ['team-chat-reader', 'messages', false], ['changes-empty', 'changes', true], ['usage-empty', 'usage', true, true],
  ['usage', 'usage', false], ['shared-tasks', 'board', false], ['team-chat-empty', 'messages', true],
];
const viewports = { desktop: { width: 1280, height: 900 }, phone: { width: 390, height: 844 } };
const phoneShots = new Set(['team-chat', 'team-chat-reader', 'usage-empty', 'shared-tasks']);
try {
  for (const [name, tab, empty, details] of shots) for (const [vname, viewport] of Object.entries(viewports)) for (const theme of ['dark', 'light']) {
    if (vname === 'phone' && !phoneShots.has(name)) continue;
    const ctx = await browser.newContext({ viewport, colorScheme: theme, reducedMotion: 'reduce' });
    const page = await ctx.newPage(); page.on('pageerror', (e) => { console.log('ERR', e.message); failed++; });
    await page.goto(`${base}?tab=${tab}${empty ? '&empty=1' : ''}${details ? '&details=1' : ''}`); await page.waitForFunction(() => window.__ready === true, null, { timeout: 30000 });
    await page.evaluate(() => document.fonts.ready); await page.waitForTimeout(500);
    if (name === 'team-chat' && theme === 'dark') {
      const m = await page.evaluate(() => [...document.querySelectorAll('[data-team-msg-body]')].map((el) => ({ lh: parseFloat(getComputedStyle(el).lineHeight), ch: el.clientHeight, sh: el.scrollHeight, more: !!el.parentElement.querySelector('.team-msg__more'), text: el.textContent })));
      check(m[0].ch <= m[0].lh * 2 + 1 && m[0].sh > m[0].ch, `${vname}: the long message is cut to two lines (${m[0].ch}px of ${m[0].sh}px, line ${m[0].lh}px)`);
      check(m[0].more && m[1].more, `${vname}: the cut messages say Show all`);
      check(!m[3].more && m[3].sh <= m[3].ch + 1, `${vname}: a short message is whole and has no Show all`);
      check(await page.locator('#inspector-panel b').count() === 0 && m[4].text.includes('<b>not bold</b>'), `${vname}: HTML in a message shows as text`);
      const spill = await page.evaluate(() => [...document.querySelectorAll('button.team-msg .team-msg__line, button.team-msg .team-msg__foot')].filter((l) => [...l.children].some((c) => c.getBoundingClientRect().right > l.getBoundingClientRect().right + 0.5 || c.getBoundingClientRect().width < 1)).length);
      check(spill === 0, `${vname}: every row's line fits: sender, recipient, tags and time all visible (${spill} spill)`);
      const before = await page.evaluate(() => getComputedStyle(document.body).overflowX);
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${vname}: no sideways scroll (${before})`);
      // The keyboard: Tab into the panel's rows, Enter opens the view, Esc closes only the view and gives the row its focus back.
      await page.locator('#inspector-panel .ide-iconbtn').first().focus(); // the panel's close button, just before the rows
      await page.keyboard.press('Tab');
      check(await page.evaluate(() => document.activeElement === document.querySelector('button.team-msg')), `${vname}: Tab reaches the first row`);
      await page.keyboard.press('Enter');
      await page.waitForSelector('[role="dialog"]');
      const full = await page.locator('[data-team-reader-body]').textContent();
      check(full === m[0].text, `${vname}: Enter opens the whole text`);
      await page.keyboard.press('Escape');
      await page.waitForSelector('[role="dialog"]', { state: 'detached' });
      await page.waitForTimeout(100);
      check(await page.locator('#inspector-panel').count() === 1, `${vname}: Esc leaves the panel open`);
      check(await page.evaluate(() => document.activeElement === document.querySelector('button.team-msg')), `${vname}: focus is back on the row`);
    }
    if (name === 'team-chat-reader') {
      await page.locator('button.team-msg').nth(1).focus();
      await page.keyboard.press('Enter');
      await page.waitForSelector('[role="dialog"]'); await page.waitForTimeout(300);
    }
    if (name !== 'team-chat-reader') await page.evaluate(() => document.activeElement?.blur());
    if (!checkOnly) {
      const file = `${out}/panels-redesign-${name}-${vname === 'desktop' ? '' : 'phone-'}${theme}.png`;
      const reader = name === 'team-chat-reader';
      const clip = vname === 'desktop' && !reader ? { x: 1280 - 336 - 44, y: 0, width: 380, height: 900 } : undefined;
      await page.screenshot({ path: file, clip, animations: 'disabled' });
      console.log('shot', file.split('/').pop());
    }
    await ctx.close();
  }
} finally { await browser.close(); await server.close(); }
process.exit(failed ? 1 : 0);
