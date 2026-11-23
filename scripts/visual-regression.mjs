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

// A page is captured once it has settled, so two runs of the same UI match (#1023): its fonts have loaded, every dwell has passed and
// the DOM is quiet. An approval's Allow waits ATTENTION_DWELL_MS (600 ms): the attention dialog's buttons are aria-disabled until then, and
// SkipApproval's "read it first" hint goes, which moves everything below it. A chat log follows its newest entry, so it sits at its
// bottom with no "Jump to latest". The real dwell is never shortened; this only waits for it, judged from the DOM rather than elapsed time
// (on a busy machine the dialog mounts late).
const QUIET_MS = 700;
async function settle(page) {
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(() => {
    window.__vrLast = performance.now();
    new MutationObserver(() => { window.__vrLast = performance.now(); }).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  });
  const start = Date.now();
  let last = -1;
  let same = 0;
  for (;;) {
    await page.waitForTimeout(150);
    const s = await page.evaluate(() => ({
      height: document.documentElement.scrollHeight,
      quiet: performance.now() - window.__vrLast,
      dwelling: !!document.querySelector('.attention-card [aria-disabled="true"], [id^="skip-wait-"]'),
      following: [...document.querySelectorAll('[role="log"]')].every((e) => e.scrollHeight - e.scrollTop - e.clientHeight <= 1)
        && !document.querySelector('.terminal-jump'),
    }));
    same = s.height === last ? same + 1 : 0;
    last = s.height;
    if (same >= 3 && s.quiet >= QUIET_MS && !s.dwelling && s.following) return;
    if (Date.now() - start > 15_000) { console.log(`     ${JSON.stringify(s)}: not settled, compared as it is`); return; }
  }
}

// A full-page capture lays the page out beyond the viewport, which resizes the window under it (innerHeight jumps and comes back): the
// attention dialog unmounts and remounts with its dwell restarted, and each chat log, whose pinned approvals are sized in vh, loses its
// place and shows "Jump to latest". Whether the page reacts before the frame is taken depends on how busy the machine is, so page script
// is held for the capture: the frame is the settled page, every time. The caret is hidden and motion is off as well.
async function capture(page, cdp) {
  await page.addStyleTag({ content: '*, *::before, *::after { caret-color: transparent !important; }' });
  await cdp.send('Emulation.setScriptExecutionDisabled', { value: true });
  try {
    return await page.screenshot({ fullPage: true, animations: 'disabled', caret: 'hide' });
  } finally {
    await cdp.send('Emulation.setScriptExecutionDisabled', { value: false });
  }
}

// The gallery's demo data is relative to the clock (an agent started 12 minutes ago, a machine heard 4 seconds ago): Date is pinned so a
// slow load or a run near a minute boundary draws the same words. Timers stay real, so every dwell still takes its full time.
const CLOCK = new Date('2026-11-20T10:00:00Z');

mkdirSync(join(visual, 'diff'), { recursive: true });
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox', '--font-render-hinting=none'] });
let failed = 0;
for (const p of pages) {
  for (const [vname, viewport] of viewports) {
    for (const scheme of ['dark', 'light']) {
      const name = `${p.name}-${vname}-${scheme}`;
      const ctx = await browser.newContext({ viewport, colorScheme: scheme, reducedMotion: 'reduce', locale: 'en-US' });
      const page = await ctx.newPage();
      const cdp = await ctx.newCDPSession(page);
      await page.clock.setFixedTime(CLOCK);
      await page.goto(base + p.hash);
      await page.waitForSelector(p.ready);
      await settle(page);
      const shot = await capture(page, cdp);
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
