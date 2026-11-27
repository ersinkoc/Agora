// Screenshots for the starter themes (roadmap M5.2, docs/ux/themes.md "Starter themes"): the visual chooser and editor picker in
// both schemes, and every starter theme applied to the window, from the
// built web app's demo window (`#/demo`, no hub). Needs a built web app and playwright-core with a Chromium:
//   cd apps/web && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots-starter-themes.mjs [dist] [outdir]
//
// A <select>'s open list is drawn by the operating system, not the page, so a browser cannot shoot it. For the picker shot the
// select is given a `size` for the moment of the shot, which draws the very same options and groups inside the page.
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
  // The demo's "Needs you" card would cover the window: these shots are of the theme (This device > Needs you > Off).
  await ctx.addInitScript(() => { try { localStorage.setItem('agora.attention.mode', 'off'); } catch { /* dismissed below */ } });
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
  if (setup) await setup(page);
  await page.waitForTimeout(400);
  return { page, ctx };
}
const shot = async (page, name) => { const f = `${out}/${name}.png`; await page.screenshot({ path: f }); console.log(f); };
const pick = (name) => async (page) => { await page.keyboard.press('Control+k'); await page.keyboard.type(`Theme: ${name}`); await page.keyboard.press('Enter'); };

// The picker: the theme of the scheme on screen, its list drawn in the page.
for (const scheme of ['dark', 'light']) {
  const { page, ctx } = await open(scheme, async (p) => {
    const select = p.locator('section[aria-label="Theme"] select').nth(scheme === 'light' ? 0 : 1);
    await select.evaluate((s) => { s.size = s.querySelectorAll('option, optgroup').length; });
  });
  await shot(page, `starter-themes-picker-${scheme}`);
  await ctx.close();
}
// The title bar's visual chooser: the same modal is used by the browser and the desktop shell.
for (const scheme of ['dark', 'light']) {
  const { page, ctx } = await open(scheme, async (p) => {
    await p.getByRole('banner').getByRole('button', { name: 'Choose theme' }).click();
  });
  await shot(page, `theme-chooser-${scheme}`);
  await ctx.close();
}
// Every starter theme on the window, in every scheme its file has.
for (const [name, scheme, file] of [
  ['Arctic', 'light', 'arctic'], ['Aurora', 'dark', 'aurora'], ['Blossom', 'light', 'blossom'],
  ['Catppuccin Mocha', 'dark', 'catppuccin-mocha'], ['Cobalt', 'dark', 'cobalt'], ['Dracula', 'dark', 'dracula'],
  ['Ember', 'dark', 'ember'], ['Gruvbox', 'dark', 'gruvbox'], ['Gruvbox', 'light', 'gruvbox'],
  ['Lavender', 'dark', 'lavender'], ['Lilac', 'light', 'lilac'], ['Mint', 'light', 'mint'],
  ['Rosé Pine', 'dark', 'rose-pine'], ['Sepia', 'light', 'sepia'], ['Tokyo Night', 'dark', 'tokyo-night'],
]) {
  const { page, ctx } = await open(scheme, pick(name));
  await shot(page, `starter-theme-${file}-${scheme}`);
  await ctx.close();
}
await browser.close();
server.close();
if (errors.length > 0) { console.error(errors.join('\n')); process.exit(1); }
