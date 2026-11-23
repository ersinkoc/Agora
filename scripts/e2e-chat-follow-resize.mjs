// #1060: a window or pane resize must not make a chat log stop following its newest entry. The component gallery draws chat logs that
// follow their newest entry; their boxes are made as tall as the window, the window is shrunk and restored (as a full-page screenshot
// does: 900 -> 1 -> 900) and the boxes are resized, and every log must still be at its bottom with no "Jump to latest". A scroll up by the
// reader, and going to a file's change, must still leave the bottom.
//
//   cd apps/web && npm ci && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-chat-follow-resize.mjs
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const dist = join(root, 'apps/web/dist');
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

const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
const failures = [];
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failures.push(what); };
try {
  const width = 1280;
  const page = await (await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: 'reduce' })).newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(`${base}#/gallery`);
  await page.waitForSelector('.gallery [role=log]');
  await page.waitForTimeout(500);
  // Every chat log: how far its scroll offset is from its bottom.
  const gaps = () => page.$$eval('[role=log]', (logs) => logs.map((l) => Math.round(l.scrollHeight - l.scrollTop - l.clientHeight)));
  // Which logs show their "Jump to latest" pill (it sits beside the log, in the same box).
  const pills = () => page.$$eval('[role=log]', (logs) => logs.map((l) => [...l.parentElement.querySelectorAll(':scope > button')].some((b) => /^Jump to latest/.test(b.textContent ?? ''))));
  const jumps = async () => (await pills()).filter(Boolean).length;
  const firstPill = async () => (await pills())[0];
  const settle = () => page.waitForTimeout(300);

  const logs = (await gaps()).length;
  check(logs >= 2, `the gallery draws chat logs (${logs})`);
  check((await gaps()).every((g) => g <= 1), `every log starts at its bottom (${await gaps()})`);
  check((await jumps()) === 0, 'no "Jump to latest" at the start');
  // The attention dialog over the gallery would take the pointer: put it away.
  await page.keyboard.press('Escape');
  await settle();

  const atBottomEverywhere = async (what) => {
    const g = await gaps();
    check(g.every((x) => x <= 1), `${what}: every log is at its bottom (${g})`);
    check((await jumps()) === 0, `${what}: no "Jump to latest"`);
  };

  // A full-page screenshot resizes the window under the page (900 -> 1 -> 900, #1059); found under load (`taskset -c 0`).
  await page.screenshot({ fullPage: true });
  await settle();
  await atBottomEverywhere('after a full-page screenshot');

  // The gallery's logs have fixed heights; in the app a log fills its pane, which follows the window. Give every log's box a height that
  // follows the window, then shrink and restore the window (a full-page screenshot does 900 -> 1 -> 900).
  await page.$$eval('[role=log]', (ls) => ls.forEach((l) => { l.parentElement.parentElement.parentElement.style.height = 'calc(100vh - 100px)'; }));
  await settle();
  await atBottomEverywhere('logs as tall as the window');
  for (const height of [1, 400, 120]) {
    await page.setViewportSize({ width, height });
    await settle();
    await page.setViewportSize({ width, height: 900 });
    await settle();
    await atBottomEverywhere(`after the window went to ${height} px and back`);
  }

  // A width change: the content reflows, so only its height changes.
  await page.setViewportSize({ width: 700, height: 900 });
  await settle();
  await atBottomEverywhere('with a narrower window');
  await page.setViewportSize({ width, height: 900 });
  await settle();
  await atBottomEverywhere('with the window wide again');

  // A pane resize: a log grows (the browser clamps its offset), then shrinks below where it was, and is restored.
  for (const [i, h] of [['grows', 'calc(100vh + 100px)'], ['shrinks', 'calc(100vh - 400px)'], ['is restored', 'calc(100vh - 100px)']]) {
    await page.$$eval('[role=log]', (ls, h) => ls.forEach((l) => { l.parentElement.parentElement.parentElement.style.height = h; }), h);
    await settle();
    await atBottomEverywhere(`after each log ${i}`);
  }

  // The reader scrolls the first log up with the wheel: it stops following and offers "Jump to latest", and a resize leaves them there.
  const first = page.locator('[role=log]').first();
  await first.scrollIntoViewIfNeeded();
  await first.hover();
  await page.mouse.wheel(0, -600);
  await settle();
  const up = (await gaps())[0];
  check(up > 64, `a scroll up by the reader leaves the bottom (${up})`);
  check((await firstPill()) && (await jumps()) === 1, 'a scroll up by the reader shows "Jump to latest" on that log only');
  await page.setViewportSize({ width, height: 600 });
  await settle();
  check((await gaps())[0] > 64 && (await firstPill()), `a resize after the reader scrolled up does not pull them back (${(await gaps())[0]})`);
  await page.setViewportSize({ width, height: 900 });
  await settle();
  await first.locator('xpath=..').getByRole('button', { name: /^Jump to latest/ }).click();
  await settle();
  await atBottomEverywhere('after "Jump to latest"');

  // "Files changed" takes the reader to a file's change above: the log stays there, not pulled back to the bottom.
  await first.getByRole('button', { name: 'auth.rs', exact: true }).click();
  await settle();
  const went = (await gaps())[0];
  check(went > 64, `going to a file's change leaves the log at that change (${went} px above the bottom)`);
  check(await firstPill(), 'going to a file\'s change shows "Jump to latest"');
  // Back to the gallery's own heights (one more pane resize for the logs that follow).
  await page.$$eval('[role=log]', (ls) => ls.forEach((l) => { l.parentElement.parentElement.parentElement.style.height = ''; }));
  await settle();
  const back = await gaps();
  check(back.slice(1).every((x) => x <= 1), `the logs that follow are at their bottom at the gallery's heights (${back})`);

  // The reader opens a fold in the last screen of the second log while it follows: the fold stays under the pointer (the log is not pinned
  // back to its bottom), and the log leaves the tail with "Jump to latest".
  const fold = await page.evaluate(() => {
    const l = document.querySelectorAll('[role=log]')[1];
    const box = l.getBoundingClientRect();
    const shown = [...l.querySelectorAll('details:not([open]) > summary')].filter((x) => { const r = x.getBoundingClientRect(); return r.height > 0 && r.top >= box.top && r.bottom <= box.bottom; });
    const summary = shown[shown.length - 1];
    if (!summary) return undefined;
    summary.setAttribute('data-e2e-fold', '');
    return Math.round(summary.getBoundingClientRect().top - box.top);
  });
  check(fold !== undefined, 'the second log shows a closed fold in its last screen');
  if (fold !== undefined) {
    await page.locator('[data-e2e-fold]').click();
    await settle();
    const moved = await page.evaluate(() => { const l = document.querySelectorAll('[role=log]')[1]; return Math.round(document.querySelector('[data-e2e-fold]').getBoundingClientRect().top - l.getBoundingClientRect().top); });
    check(moved === fold, `an opened fold stays where the reader clicked it (${fold} -> ${moved} px from the log's top)`);
    check((await pills())[1] === ((await gaps())[1] > 64), `the log offers "Jump to latest" exactly when the fold took it off its bottom (${(await gaps())[1]} px)`);
  }
  check(errors.length === 0, `no page errors (${errors.join('; ')})`);
} finally {
  await browser.close();
  server.close();
}
if (failures.length) { console.log(`${failures.length} failed`); process.exit(1); }
console.log('all passed');
