// Screenshots for theme import (docs/ux/themes.md "Import from another app"): the import dialog over the theme editor in both schemes,
// from the built web app's demo window (`#/demo`, no hub), and a check that importing an iTerm2 file (whose DOCTYPE names Apple's DTD
// on the web) makes no request at all. Needs a built web app and playwright-core with a Chromium:
//   cd apps/web && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots-theme-import.mjs [dist] [outdir]
import { createServer } from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { extname, join } from 'node:path';
const dist = process.argv[2] ?? new URL('../apps/web/dist/', import.meta.url).pathname;
const out = process.argv[3] ?? new URL('../docs/screenshots', import.meta.url).pathname;
const fixtures = new URL('../apps/web/src/themes/import/fixtures/', import.meta.url).pathname;
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
async function editor(scheme) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
  await ctx.addInitScript(() => { try { localStorage.setItem('agora.attention.mode', 'off'); } catch { /* the card is dismissed below */ } });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);
  await page.waitForSelector('.ide');
  if (await page.locator('.attention-card').waitFor({ timeout: 1500 }).then(() => true, () => false)) {
    await page.waitForTimeout(900);
    await page.keyboard.press('Escape');
    await page.locator('.attention-card').waitFor({ state: 'detached' });
  }
  await page.getByRole('button', { name: 'Layout' }).click();
  await page.getByRole('menuitemradio', { name: /^One pane/i }).click();
  await page.keyboard.press('Control+k');
  await page.keyboard.type('Edit theme');
  await page.keyboard.press('Enter');
  await page.waitForSelector('section[aria-label="Theme"]');
  return { page, ctx };
}
const importFile = async (page, file, name) => {
  await page.locator('[data-import-other]').setInputFiles({ name, mimeType: 'application/octet-stream', buffer: readFileSync(join(fixtures, file)) });
  await page.waitForSelector('[data-theme-import]');
  await page.waitForTimeout(300);
};

// The dialog in both schemes: a dark VS Code theme in the dark window, a light Ghostty theme in the light one.
for (const [scheme, file, name] of [['dark', 'harbour-color-theme.json', 'harbour-color-theme.json'], ['light', 'harbour-light', 'Harbour Light']]) {
  const { page, ctx } = await editor(scheme);
  await importFile(page, file, name);
  const f = `${out}/theme-import-${scheme}.png`;
  await page.screenshot({ path: f });
  console.log(f);
  // Added: the window takes the theme of that scheme.
  await page.getByRole('button', { name: 'Add theme' }).click();
  await page.waitForSelector('[data-theme-import]', { state: 'detached' });
  await page.waitForTimeout(300);
  const g = `${out}/theme-import-added-${scheme}.png`;
  await page.screenshot({ path: g });
  console.log(g);
  await ctx.close();
}

// No request leaves the page while an iTerm2 file is read (its DOCTYPE points at apple.com).
{
  const { page, ctx } = await editor('dark');
  const requests = [];
  page.on('request', (r) => requests.push(r.url()));
  await importFile(page, 'harbour.itermcolors', 'harbour.itermcolors');
  await page.getByRole('button', { name: 'Add theme' }).click();
  await page.waitForTimeout(500);
  console.log(`requests while importing an iTerm2 file: ${requests.length}${requests.length ? ` ${requests.join(' ')}` : ''}`);
  if (requests.some((r) => !r.startsWith(url.replace('/#/demo', '')))) errors.push(`a request left the page: ${requests.join(' ')}`);
  await ctx.close();
}
await browser.close();
server.close();
if (errors.length) { console.log('PAGE ERRORS', errors); process.exit(1); }
