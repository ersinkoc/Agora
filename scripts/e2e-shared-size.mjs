// End-to-end check of a terminal watched from two windows (docs/specs/protocol.md "Shared terminal size") in a real browser: a
// shell opened in a 1600 × 900 window and in a 900 × 600 one has the smaller window's size in both (the shell's own `stty size`
// agrees), the bigger window shows it at its top left without asking for more, and nothing keeps resizing; when the small window
// closes the big one gets its own size back, and when it opens again both shrink again.
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-shared-size.mjs
//   AGORA_SCREENSHOTS=<dir> node scripts/e2e-shared-size.mjs   # also writes shared-size-*.png in both themes
//
// Needs git, sh and a Chromium (CHROME=<path>, default: Playwright's cache). Uses a throw-away folder; changes nothing else.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const shots = process.env.AGORA_SCREENSHOTS;
if (shots) mkdirSync(shots, { recursive: true });
let stack;

// The size a window shows the terminal at (the node's), and whether it is smaller than what fits that window.
const shown = (page) => page.evaluate(() => {
  const t = document.querySelector('.terminal-host[data-cols]');
  return t ? { cols: Number(t.dataset.cols), rows: Number(t.dataset.rows), smaller: t.hasAttribute('data-smaller') } : undefined;
});
const settle = async (page, want, what) => {
  try { await page.waitForFunction(want, null, { timeout: 15000 }); } catch { console.log(`  waiting for ${what}: ${JSON.stringify(await shown(page))}`); }
};
const screen = (page) => page.evaluate(() => document.querySelector('.xterm-rows')?.innerText ?? '');
// The shell's own idea of its size: `stty size` prints "rows cols".
const stty = async (page, tag) => {
  await page.focus('.xterm-helper-textarea');
  await page.keyboard.type(`clear; echo ${tag} $(stty size) ${tag}\n`);
  await page.waitForFunction((t) => new RegExp(`${t} \\d+ \\d+ ${t}`).test(document.querySelector('.xterm-rows')?.innerText ?? ''), tag, { timeout: 10000 });
  const m = (await screen(page)).match(new RegExp(`${tag} (\\d+) (\\d+) ${tag}`));
  return { rows: Number(m[1]), cols: Number(m[2]) };
};
// Frames a window sends the hub during `ms` (a terminal that keeps resizing shows up here; pings are every 15 s).
const sentDuring = async (page, ms) => {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Network.enable');
  let n = 0;
  cdp.on('Network.webSocketFrameSent', () => { n++; });
  await page.waitForTimeout(ms);
  await cdp.detach();
  return n;
};

try {
  stack = await startStack({ viewport: { width: 1600, height: 900 } });
  const { page: big, start, errors } = stack;
  await start('Shell');
  const agentUrl = big.url();
  await big.waitForFunction(() => /\$\s*$/.test((document.querySelector('.xterm-rows')?.innerText ?? '').trim()), null, { timeout: 30000 });
  await settle(big, () => !!document.querySelector('.terminal-host[data-cols]'), 'big alone');
  const alone = await shown(big);
  check(!!alone && !alone.smaller, `alone, the big window's terminal fits it (${alone?.cols}×${alone?.rows})`);

  const openSmall = async () => {
    const small = await big.context().newPage();
    small.on('pageerror', (e) => errors.push(e.message));
    await small.setViewportSize({ width: 900, height: 600 });
    await small.goto(agentUrl);
    await small.waitForSelector('.terminal-host[data-cols]', { timeout: 15000 });
    await settle(small, () => !document.querySelector('.terminal-host[data-smaller]'), 'small');
    return small;
  };

  for (const round of ['first', 'again']) {
    const small = await openSmall();
    const s = await shown(small);
    await big.waitForFunction((w) => { const t = document.querySelector('.terminal-host[data-cols]'); return t && t.dataset.cols === String(w.cols) && t.dataset.rows === String(w.rows); }, s, { timeout: 15000 }).catch(() => console.log('  the big window did not take the small size'));
    const b = await shown(big);
    check(!!s && !s.smaller && s.cols < alone.cols && s.rows < alone.rows, `${round}: the small window's terminal fits it (${s?.cols}×${s?.rows})`);
    check(!!b && b.cols === s.cols && b.rows === s.rows && b.smaller, `${round}: the big window shows the same, smaller terminal (${b?.cols}×${b?.rows})`);
    const real = await stty(big, `sz${round}`);
    check(real.cols === s.cols && real.rows === s.rows, `${round}: the shell agrees (${real.cols}×${real.rows})`);
    // The bigger window does not ask for more, and the smaller does not ask again: nothing goes back and forth.
    const quiet = await Promise.all([sentDuring(big, 3000), sentDuring(small, 3000)]);
    check(quiet[0] <= 1 && quiet[1] <= 1, `${round}: no resize loop (frames sent in 3 s: ${quiet.join(', ')})`);
    // The terminal sits at the top left of the bigger window; nothing scrolls sideways.
    const box = await big.evaluate(() => {
      const t = document.querySelector('.terminal-host'); const sc = document.querySelector('.xterm-screen');
      const a = t.getBoundingClientRect(); const z = sc.getBoundingClientRect();
      return { left: z.left - a.left, top: z.top - a.top, right: a.right - z.right, bottom: a.bottom - z.bottom, sideways: document.scrollingElement.scrollWidth > innerWidth };
    });
    check(box.left < 20 && box.top < 20 && box.right > 100 && box.bottom > 50 && !box.sideways, `${round}: top left with room around it (${JSON.stringify(box)})`);
    if (shots && round === 'first') {
      for (const scheme of ['light', 'dark']) {
        await big.emulateMedia({ colorScheme: scheme });
        await small.emulateMedia({ colorScheme: scheme });
        await big.waitForTimeout(400);
        await big.screenshot({ path: join(shots, `shared-size-big-${scheme}.png`) });
        await small.screenshot({ path: join(shots, `shared-size-small-${scheme}.png`) });
      }
      await big.emulateMedia({ colorScheme: null });
    }
    await small.close();
    await big.waitForFunction((w) => { const t = document.querySelector('.terminal-host[data-cols]'); return t && t.dataset.cols === String(w.cols) && t.dataset.rows === String(w.rows); }, alone, { timeout: 15000 }).catch(() => console.log('  the big window did not grow back'));
    const back = await shown(big);
    check(!!back && back.cols === alone.cols && back.rows === alone.rows && !back.smaller, `${round}: with the small window closed the big one grows back (${back?.cols}×${back?.rows})`);
    const real2 = await stty(big, `bk${round}`);
    check(real2.cols === alone.cols && real2.rows === alone.rows, `${round}: the shell agrees (${real2.cols}×${real2.rows})`);
  }
  check(errors.length === 0, `no page errors (${errors.join('; ')})`);
} catch (e) {
  console.log(`FAIL ${e.stack ?? e}`);
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
