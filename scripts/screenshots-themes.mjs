// Screenshots for ADR-0068 rung 2 (docs/ux/themes.md): the theme editor in both schemes, four themes, and a change refused for
// contrast, from the built web app's demo window (`#/demo`, no hub). Needs a built web app and playwright-core with a Chromium:
//   cd apps/web && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots-themes.mjs [dist] [outdir]
import { createServer } from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { extname, join } from 'node:path';
const dist = process.argv[2] ?? new URL('../apps/web/dist/', import.meta.url).pathname;
const out = process.argv[3] ?? new URL('../docs/screenshots', import.meta.url).pathname;
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm' };
const server = createServer((req, res) => {
  const path = join(dist, req.url === '/' || !req.url ? 'index.html' : req.url.split('?')[0]);
  if (!existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const url = `http://127.0.0.1:${server.address().port}/#/demo`;
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
const errors = [];
async function open(scheme, setup) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);
  await page.waitForSelector('.ide');
  // Two agents side by side, then the editor from the command palette.
  await page.getByRole('button', { name: 'Layout' }).click();
  await page.getByRole('menuitem', { name: /^One pane/i }).click();
  await page.keyboard.press('Control+k');
  await page.keyboard.type('Edit theme');
  await page.keyboard.press('Enter');
  await page.waitForSelector('section[aria-label="Theme"]');
  if (setup) await setup(page);
  await page.waitForTimeout(400);
  return { page, ctx };
}
const shot = async (page, name) => { const f = `${out}/${name}.png`; await page.screenshot({ path: f }); console.log(f); };
const pick = (name) => async (page) => { await page.keyboard.press('Control+k'); await page.keyboard.type(`Theme: ${name}`); await page.keyboard.press('Enter'); };

for (const scheme of ['dark', 'light']) { const { page, ctx } = await open(scheme); await shot(page, `adr68-theme-editor-${scheme}`); await ctx.close(); }
for (const [name, scheme, file] of [['High contrast', 'dark', 'high-contrast'], ['Large text', 'dark', 'large-text'], ['Slate', 'dark', 'slate'], ['Sand', 'light', 'sand']]) {
  const { page, ctx } = await open(scheme, pick(name)); await shot(page, `adr68-theme-${file}-${scheme}`); await ctx.close();
}
// A customized theme with text that would be hard to read: warned, not applied.
for (const scheme of ['dark', 'light']) {
  const { page, ctx } = await open(scheme, async (p) => {
    await p.getByRole('button', { name: 'Customize' }).click();
    const text = p.locator('label', { hasText: /^Secondary text$/ });
    const id = await text.getAttribute('for');
    await p.locator(`[id="${id}"]`).fill(scheme === 'dark' ? '#3a3a40' : '#c8c8d0');
    await p.locator('section[aria-label="Theme"]').getByText('Readability').scrollIntoViewIfNeeded();
  });
  await shot(page, `adr68-theme-contrast-warning-${scheme}`); await ctx.close();
}
await browser.close();
server.close();
if (errors.length) { console.log('PAGE ERRORS', errors); process.exit(1); }
