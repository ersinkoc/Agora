// Visual regression: the component gallery and the sign-in page, in both themes on a desktop and a phone viewport, compared with
// the committed baselines in apps/web/visual/. A pull request that changes how a screen looks updates them on purpose.
//
//   cd apps/web && npm ci && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/visual-regression.mjs            # compare, exit 1 on a difference
//   PW_DIR=... node scripts/visual-regression.mjs --update                                       # rewrite the baselines
//
// Differences are written to apps/web/visual/diff/ (ignored by git). Baselines come from headless Chromium on Linux with the
// system font stack, so they are compared on that setup; another OS or font set will differ and should rewrite its own.
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { extname, join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'apps/web/dist');
const visual = join(root, 'apps/web/visual');
const update = process.argv.includes('--update');
const tolerance = Number(process.env.VISUAL_TOLERANCE ?? 0.0005); // share of pixels allowed to differ
const web = createRequire(join(root, 'apps/web/package.json'));
const { PNG } = web('pngjs');
const pixelmatch = (await import(web.resolve('pixelmatch'))).default;
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm' };

const server = createServer((req, res) => {
  const path = join(dist, !req.url || req.url === '/' ? 'index.html' : req.url.split('?')[0]);
  if (!existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const base = `http://127.0.0.1:${server.address().port}/`;

const pages = [
  { name: 'gallery', hash: '#/gallery', ready: '.gallery' },
  { name: 'signin', hash: '', ready: 'form' },
];
const viewports = [['desktop', { width: 1280, height: 900 }], ['phone', { width: 390, height: 844 }]];

mkdirSync(join(visual, 'diff'), { recursive: true });
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox', '--font-render-hinting=none'] });
let failed = 0;
for (const p of pages) {
  for (const [vname, viewport] of viewports) {
    for (const scheme of ['dark', 'light']) {
      const name = `${p.name}-${vname}-${scheme}`;
      const ctx = await browser.newContext({ viewport, colorScheme: scheme, reducedMotion: 'reduce' });
      const page = await ctx.newPage();
      await page.goto(base + p.hash);
      await page.waitForSelector(p.ready);
      await page.waitForTimeout(300);
      const shot = await page.screenshot({ fullPage: true, animations: 'disabled' });
      await ctx.close();
      const file = join(visual, `${name}.png`);
      if (update || !existsSync(file)) {
        writeFileSync(file, shot);
        console.log(`${update ? 'updated' : 'created'} ${name}`);
        continue;
      }
      const a = PNG.sync.read(readFileSync(file));
      const b = PNG.sync.read(shot);
      if (a.width !== b.width || a.height !== b.height) {
        console.log(`FAIL ${name}: size ${a.width}x${a.height} became ${b.width}x${b.height}`);
        writeFileSync(join(visual, 'diff', `${name}.actual.png`), shot);
        failed++;
        continue;
      }
      const diff = new PNG({ width: a.width, height: a.height });
      const n = pixelmatch(a.data, b.data, diff.data, a.width, a.height, { threshold: 0.1 });
      const share = n / (a.width * a.height);
      if (share > tolerance) {
        console.log(`FAIL ${name}: ${n} pixels differ (${(share * 100).toFixed(2)}%, allowed ${(tolerance * 100).toFixed(2)}%)`);
        writeFileSync(join(visual, 'diff', `${name}.diff.png`), PNG.sync.write(diff));
        writeFileSync(join(visual, 'diff', `${name}.actual.png`), shot);
        failed++;
      } else {
        console.log(`ok   ${name} (${n} pixels differ)`);
      }
    }
  }
}
await browser.close();
server.close();
process.exit(failed ? 1 : 0);
