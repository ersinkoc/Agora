// Screenshots for ADR-0068 rung 2 (docs/ux/themes.md): the theme editor in both schemes, four themes, a change refused for
// contrast, and changes applied with warnings (text on a coloured background, #1021; the kit's other colours, #1054), from the
// built web app's demo window (`#/demo`, no hub). Needs a built web app and playwright-core with a Chromium:
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
  // The demo has an agent that needs you, and its "Needs you" card covers the window (and makes the rest of it inert). These shots
  // are of the theme, so this browser asks for no card or banner (This device > Needs you > Off, `agora.attention.mode`).
  await ctx.addInitScript(() => { try { localStorage.setItem('agora.attention.mode', 'off'); } catch { /* the card is dismissed below */ } });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);
  await page.waitForSelector('.ide');
  // If the card shows anyway, wait out its dwell and answer Later (Esc), so it does not cover the shot.
  if (await page.locator('.attention-card').waitFor({ timeout: 1500 }).then(() => true, () => false)) {
    await page.waitForTimeout(900);
    await page.keyboard.press('Escape');
    await page.locator('.attention-card').waitFor({ state: 'detached' });
  }
  // One pane, then the editor from the command palette (it opens beside it).
  await page.getByRole('button', { name: 'Layout' }).click();
  await page.getByRole('menuitemradio', { name: /^One pane/i }).click(); // the presets are a row of tiles (radio items)
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
// A customized theme applied with warnings, shot as the editor's own pane scrolled to Readability: text on a coloured background
// (#1021), and the kit's other colours (#1054: an agent's badge, a role's icon on its chip).
const custom = (values) => async (p) => {
  await p.getByRole('button', { name: 'Customize' }).click();
  for (const [label, value] of values) {
    const id = await p.locator('label', { hasText: new RegExp(`^${label}$`) }).getAttribute('for');
    await p.locator(`[id="${id}"]`).fill(value);
  }
  await p.locator('section[aria-label="Theme"]').getByText('Readability').scrollIntoViewIfNeeded();
};
const pane = (page) => page.locator('section[aria-label="Theme"]').locator('xpath=ancestor::*[contains(@class, "pane") or @data-pane][1]');
for (const [file, scheme, values] of [
  ['theme-tint-warning', 'dark', [['Idle', '#707078']]], ['theme-tint-warning', 'light', [['Idle', '#80808a']]],
  ['theme-kit-warning', 'dark', [['Text', '#9c9ca4'], ['Idle', '#66666e']]], ['theme-kit-warning', 'light', [['Idle', '#868692']]],
]) {
  const { page, ctx } = await open(scheme, custom(values));
  const f = `${out}/${file}-${scheme}.png`;
  await ((await pane(page).count()) ? pane(page) : page.locator('section[aria-label="Theme"]')).screenshot({ path: f });
  console.log(f);
  await ctx.close();
}
await browser.close();
server.close();
if (errors.length) { console.log('PAGE ERRORS', errors); process.exit(1); }
