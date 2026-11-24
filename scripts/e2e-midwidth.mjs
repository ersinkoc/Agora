// Middle widths (700–1023 px, issue #1069): the Agents list stacks above the panes there, and its Group by control must be fully
// visible and pressable, not covered by the panes' tab bar. Uses the built web app's demo window (`#/demo`, no hub).
//
//   cd apps/web && npm ci && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-midwidth.mjs
//   AGORA_SCREENSHOTS=docs/ux/screens node scripts/e2e-midwidth.mjs   # also writes midwidth-900-{dark,light}.png (900 × 650)
//
// Needs a Chromium (CHROME=<path>, default: Playwright's cache). Changes nothing else.
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
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
const url = `http://127.0.0.1:${server.address().port}/#/demo`;
const LIST = 'aside[aria-label="Agents"]';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const shots = process.env.AGORA_SCREENSHOTS;
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
const errors = [];
try {
  // A short window (650 px high) is where the panes used to squeeze the list until its Group by row slid under their tab bar.
  for (const [width, height] of [760, 900, 1000].flatMap((w) => [[w, 650], [w, 800]])) {
    for (const scheme of shots && width === 900 && height === 650 ? ['dark', 'light'] : ['dark']) {
      const at = `${width}×${height} ${scheme}`;
      const ctx = await browser.newContext({ viewport: { width, height }, colorScheme: scheme, reducedMotion: 'reduce' });
      const page = await ctx.newPage();
      page.on('pageerror', (e) => errors.push(String(e)));
      await page.goto(url);
      await page.waitForSelector('.ide');
      // The demo asks something at once (Esc is Later); under 1024 px the agent list starts hidden.
      await page.getByRole('button', { name: 'Later' }).waitFor({ timeout: 5000 }).then(() => page.keyboard.press('Escape'), () => {});
      await page.getByRole('button', { name: 'Show the agent list' }).click();
      await page.waitForSelector(LIST);
      await page.waitForTimeout(300);
      const group = page.locator(`${LIST} [role=radiogroup][aria-label="Group by"]`);
      // Every point of every Group by button (centre and corners, 2 px in) belongs to that button, nothing paints over it.
      const covered = await group.evaluate((el) => {
        const out = [];
        for (const b of el.querySelectorAll('button')) {
          const r = b.getBoundingClientRect();
          const points = [[r.left + r.width / 2, r.top + r.height / 2], [r.left + 2, r.top + 2], [r.right - 2, r.top + 2], [r.left + 2, r.bottom - 2], [r.right - 2, r.bottom - 2]];
          for (const [x, y] of points) {
            const hit = document.elementFromPoint(x, y);
            if (!hit || !b.contains(hit)) out.push(`${b.textContent?.trim()} at ${Math.round(x)},${Math.round(y)} under ${hit ? hit.tagName.toLowerCase() + (hit.className && typeof hit.className === 'string' ? '.' + hit.className.split(' ')[0] : '') : 'nothing'}`);
          }
          if (r.bottom > window.innerHeight || r.top < 0) out.push(`${b.textContent?.trim()} outside the window`);
        }
        return out;
      });
      check(covered.length === 0, `${at}: every Group by button is fully visible${covered.length ? ` (${covered.slice(0, 3).join('; ')})` : ''}`);
      // The list has a height of its own: nothing of it spills out of its box (under the panes), the panes' tab bar starts below it,
      // and its agents get room for a few rows (they scroll inside it).
      const fit = await page.evaluate((sel) => {
        const aside = document.querySelector(sel);
        const nav = aside.querySelector('nav');
        const tab = document.querySelector('main [role=tab]')?.getBoundingClientRect();
        return { spill: aside.scrollHeight - aside.clientHeight, below: tab ? Math.round(tab.top - aside.getBoundingClientRect().bottom) : null, rows: nav.clientHeight };
      }, LIST);
      check(fit.spill <= 1, `${at}: nothing of the Agents list spills out of it (${fit.spill} px)`);
      check(fit.below !== null && fit.below >= 0, `${at}: the tab bar starts below the Agents list (${fit.below} px)`);
      check(fit.rows >= 84, `${at}: the agents in the list have room for a few rows (${fit.rows} px)`);
      // Pressable: a real click at a button's centre chooses it.
      const buttons = group.locator('button');
      const n = await buttons.count();
      const target = buttons.nth(n - 1);
      const box = await target.boundingBox();
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
      // Group by is the kit's segmented control: a radio group, so the chosen button is aria-checked.
      const pressed = await target.getAttribute('aria-checked');
      check(pressed === 'true', `${at}: clicking the last Group by button chooses it`);
      await buttons.nth(0).click({ trial: true, timeout: 2000 }).then(() => true, () => false).then((ok) => check(ok, `${at}: the first Group by button can be clicked`));
      if (shots && width === 900 && height === 650) {
        await buttons.nth(0).click();
        await page.mouse.move(0, 0);
        await page.waitForTimeout(200);
        mkdirSync(shots, { recursive: true });
        await page.screenshot({ path: join(shots, `midwidth-900-${scheme}.png`), animations: 'disabled' });
      }
      await ctx.close();
    }
  }
  // The desktop keeps its layout: the list sits beside the panes from 1024 px; under 700 px a phone has a window of its own.
  {
    const ctx = await browser.newContext({ viewport: { width: 1024, height: 650 } });
    const page = await ctx.newPage();
    await page.goto(url);
    await page.waitForSelector(LIST);
    const side = await page.evaluate((sel) => {
      const a = document.querySelector(sel).getBoundingClientRect();
      const m = document.querySelector('.ide main').getBoundingClientRect();
      return a.right <= m.left + 1 && Math.abs(a.top - m.top) < 2;
    }, LIST);
    check(side, '1024 px: the Agents list sits beside the panes');
    await ctx.close();
  }
  {
    const ctx = await browser.newContext({ viewport: { width: 600, height: 800 } });
    const page = await ctx.newPage();
    await page.goto(url);
    await page.waitForSelector('.phone');
    check(await page.evaluate((sel) => !document.querySelector('.ide') && !document.querySelector(sel), LIST), '600 px: the phone window, without the stacked list');
    await ctx.close();
  }
} finally {
  await browser.close();
  server.close();
}
if (errors.length) { console.log('PAGE ERRORS', errors); failed = true; }
process.exit(failed ? 1 : 0);
