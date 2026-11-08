// Screenshots of the component gallery (or ROUTE=device etc.) (both themes, desktop and phone) for UI pull requests.
// Needs a built web app (`cd apps/web && npm run build`) and playwright-core with a Chromium:
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots.mjs [outdir]
import { createServer } from 'node:http';
import { readFileSync, mkdirSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { extname, join } from 'node:path';

const dist = new URL('../apps/web/dist/', import.meta.url).pathname;
const out = process.argv[2] ?? 'screenshots';
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm' };

const server = createServer((req, res) => {
  const path = join(dist, req.url === '/' || !req.url ? 'index.html' : req.url.split('?')[0]);
  if (!existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const url = `http://127.0.0.1:${server.address().port}/#/${process.env.ROUTE ?? 'gallery'}`;

mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
for (const [name, viewport] of [['desktop', { width: 1280, height: 900 }], ['phone', { width: 390, height: 844 }]]) {
  for (const scheme of ['dark', 'light']) {
    const page = await (await browser.newContext({ viewport, colorScheme: scheme })).newPage();
    await page.goto(url);
    await page.waitForSelector(process.env.ROUTE ? '.page' : '.gallery');
    const file = `${out}/${process.env.ROUTE ?? 'gallery'}-${name}-${scheme}.png`;
    await page.screenshot({ path: file, fullPage: true });
    console.log(file);
  }
}
await browser.close();
server.close();
