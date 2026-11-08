// Client-side performance in a real browser (ADR-0022 budgets, docs/design/performance.md): keystroke to visible echo through
// the hub and node on this machine, and the main thread under a heavy output flood.
//
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/perf-browser.mjs [flood megabytes, default 20]
import { startStack } from './browser-harness.mjs';

const megabytes = Number(process.argv[2] ?? 20);
const pct = (xs, p) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor((p / 100) * xs.length))];
let stack;
try {
  stack = await startStack();
  const { page, start } = stack;
  await start('Shell');
  await page.waitForSelector('.xterm-helper-textarea', { state: 'attached', timeout: 10000 });
  await page.focus('.xterm-helper-textarea');
  await page.waitForTimeout(1000);

  // Keystroke -> echo: a synthetic key press on xterm's input, until the next animation frame after the screen text grew.
  const echo = await page.evaluate(async (n) => {
    const input = document.querySelector('.xterm-helper-textarea');
    const rows = document.querySelector('.xterm-rows');
    const text = () => rows.innerText.length;
    const out = [];
    for (let i = 0; i < n; i++) {
      const before = text();
      const t0 = performance.now();
      const done = new Promise((resolve) => {
        const mo = new MutationObserver(() => {
          if (text() > before) {
            mo.disconnect();
            const dom = performance.now() - t0;
            requestAnimationFrame(() => resolve([dom, performance.now() - t0]));
          }
        });
        mo.observe(rows, { subtree: true, childList: true, characterData: true });
        setTimeout(() => { mo.disconnect(); resolve([-1, -1]); }, 2000);
      });
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', keyCode: 65, bubbles: true }));
      out.push(await done);
      await new Promise((r) => setTimeout(r, 20));
    }
    return out;
  }, 100);
  const line = (name, xs) => `${name} (${xs.length}/${echo.length} samples): p50 ${pct(xs, 50).toFixed(1)} ms, p95 ${pct(xs, 95).toFixed(1)} ms, max ${Math.max(...xs).toFixed(1)} ms`;
  console.log(line('keystroke -> screen text updated', echo.map((e) => e[0]).filter((x) => x >= 0)));
  console.log(line('keystroke -> next animation frame  ', echo.map((e) => e[1]).filter((x) => x >= 0)) + '   [budget p95 <= 16 ms + network; headless frame pacing adds to this]');

  // Flood: output that never pauses, watching for long main-thread tasks and how fast it drains.
  await page.evaluate(() => {
    window.__long = [];
    new PerformanceObserver((l) => l.getEntries().forEach((e) => window.__long.push(e.duration))).observe({ entryTypes: ['longtask'] });
  });
  await page.keyboard.press('Control+u');
  const t0 = Date.now();
  await page.keyboard.type(`yes 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef | head -c ${megabytes * 1_000_000}; echo FLOOD-DONE\n`, { delay: 1 });
  await page.waitForFunction(() => /FLOOD-DONE/.test(document.querySelector('.xterm-rows')?.innerText ?? '') && !/yes 0123/.test(document.querySelector('.xterm-rows')?.innerText.split('FLOOD-DONE')[1] ?? ''), null, { timeout: 120000 }).catch(() => {});
  const secs = (Date.now() - t0) / 1000;
  const long = await page.evaluate(() => window.__long);
  console.log(`flood ${megabytes} MB: drained in ${secs.toFixed(1)} s (${(megabytes / secs).toFixed(1)} MB/s), long main-thread tasks: ${long.length}, longest ${long.length ? Math.max(...long).toFixed(0) : 0} ms   [budget: no task > 50 ms at 10 MB/s]`);
} finally {
  await stack?.close();
}
