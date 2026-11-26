// Reload on save in a real browser (roadmap M5.7; docs/ux/plugins.md, "Reload on save"), with screenshots in both schemes. The built web
// app's demo window (`#/demo`, no hub) is served under the hub's headers per path (scripts/hub-headers.mjs), and the File System Access
// picker is stood in for by one whose file this script changes. It checks that:
// - a plugin loaded through the picker is listed as watched (screenshot: plugin-reload-dialog-<scheme>.png);
// - a changed package that passes starts a new frame, which boots and draws the new version and is inert until clicked into;
// - a changed package that fails keeps the frame that was running, and the pane says why above it (plugin-reload-refused-<scheme>.png).
//   cd apps/web && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots-plugin-reload.mjs [dist] [outdir]
import { createServer } from 'node:http';
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';
import { hubHeaders } from './hub-headers.mjs';
import { pack } from './pack-plugin.mjs';

const root = new URL('..', import.meta.url).pathname;
const dist = process.argv[2] ?? join(root, 'apps/web/dist');
const out = process.argv[3] ?? join(root, 'docs/screenshots');
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml' };

// The sample plugin, and a second version of it whose heading says it was reloaded.
const sample = join(root, 'apps/web/plugins/sample');
const first = pack(sample);
const work = mkdtempSync(join(tmpdir(), 'agora-reload-'));
cpSync(sample, work, { recursive: true });
writeFileSync(join(work, 'panel.js'), readFileSync(join(sample, 'panel.js'), 'utf8').replace("title.textContent = 'Agents';", "title.textContent = 'Agents, saved again';"));
const second = pack(work, { writeHashes: true });
// A third that the window refuses: panel.js changed without its hash.
writeFileSync(join(work, 'panel.js'), `${readFileSync(join(work, 'panel.js'), 'utf8')}\n// edited`);
const broken = JSON.stringify({ 'agora-plugin-bundle': 1, files: { ...JSON.parse(second).files, 'panel.js': readFileSync(join(work, 'panel.js')).toString('base64') } });
rmSync(work, { recursive: true, force: true });

const server = createServer((req, res) => {
  const url = (req.url ?? '/').split('?')[0];
  const path = join(dist, url === '/' ? 'index.html' : url);
  if (!existsSync(path)) { res.writeHead(404, hubHeaders(url, false)).end(); return; }
  res.writeHead(200, { ...hubHeaders(url), 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const url = `http://127.0.0.1:${server.address().port}/#/demo`;
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
const errors = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) errors.push(what); };

for (const scheme of ['dark', 'light']) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript((pkg) => {
    try {
      localStorage.setItem('agora.attention.mode', 'off');
      localStorage.setItem('agora.workspace.v1', JSON.stringify({ version: 1, layout: { version: 1, root: { type: 'split', direction: 'row', ratios: [0.6, 0.4], children: [{ type: 'pane', id: 'p1', panel: 'plugin.sample-agents.list' }, { type: 'pane', id: 'p2', panel: 'usage' }] } }, focus: 'p1', touched: true }));
    } catch { /* the defaults then */ }
    // The picker, standing in for the browser's: its one file is what this script last wrote, changed at `__at`.
    window.__pkg = pkg;
    window.__at = 1;
    window.showOpenFilePicker = async () => [{ getFile: async () => new File([window.__pkg], 'sample-agents.json', { type: 'application/json', lastModified: window.__at }) }];
  }, first);
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);
  await page.waitForSelector('.ide');
  await page.keyboard.press('Control+k');
  await page.keyboard.type('Plugins');
  await page.keyboard.press('Enter');
  await page.waitForSelector('[data-plugins-dialog]');
  check(await page.locator('[data-plugin-reload-unsupported]').count() === 0, `${scheme}: Chromium is offered reload on save`);
  await page.getByRole('button', { name: 'Load from a file…' }).click();
  await page.waitForSelector('[data-plugin-grants] input[type=checkbox]');
  for (const box of await page.locator('[data-plugin-grants] input[type=checkbox]').all()) await box.check();
  await page.locator('[data-plugin-save]').click();
  await page.waitForSelector('[data-plugin-reload="sample-agents"]');
  await page.waitForTimeout(300);
  const dialogShot = join(out, `plugin-reload-dialog-${scheme}.png`);
  await page.screenshot({ path: dialogShot });
  console.log(dialogShot);
  await page.keyboard.press('Escape');

  // The first version draws.
  const frame = () => page.frameLocator('iframe[data-plugin-frame="sample-agents"]');
  await frame().locator('body[data-ready] h1').waitFor();
  check(await frame().locator('h1').textContent() === 'Agents', `${scheme}: the first version draws`);
  check(await frame().locator('li').count() > 0, `${scheme}: it is granted and lists the agents`);
  const before = await page.locator('iframe[data-plugin-frame]').elementHandle();
  await page.locator('[data-plugin-box]').click();
  check(await page.locator('iframe[data-plugin-frame]').getAttribute('inert') === null, `${scheme}: the frame is in use after a click`);

  // Saved again: a new frame with the new version, inert.
  await page.evaluate((pkg) => { window.__pkg = pkg; window.__at = 2; }, second);
  await frame().locator('h1', { hasText: 'Agents, saved again' }).waitFor({ timeout: 10_000 });
  check(await before.evaluate((e) => !e.isConnected), `${scheme}: the old frame is gone`);
  check(await page.locator('iframe[data-plugin-frame]').getAttribute('inert') !== null, `${scheme}: the new frame is inert`);
  const running = await page.locator('iframe[data-plugin-frame]').elementHandle();

  // Saved with a mistake: the frame that was running stays, and the pane says why.
  await page.evaluate((pkg) => { window.__pkg = pkg; window.__at = 3; }, broken);
  await page.waitForSelector('[data-plugin-reload-problem="sample-agents"]', { timeout: 10_000 });
  check(await running.evaluate((e) => e.isConnected), `${scheme}: the running frame stays when a change is refused`);
  const note = await page.locator('[data-plugin-reload-problem]').textContent();
  check(/panel\.js/.test(note ?? ''), `${scheme}: the note names the file (${note})`);
  await page.mouse.click(5, 5);
  await page.waitForTimeout(300);
  const paneShot = join(out, `plugin-reload-refused-${scheme}.png`);
  await page.screenshot({ path: paneShot });
  console.log(paneShot);
  await ctx.close();
}
await browser.close();
server.close();
if (errors.length) { console.log('FAILED', errors); process.exit(1); }
