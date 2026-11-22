// The terminal repaint carries the program's modes (ADR-0100 step 1), in a real browser and in the terminal client. A probe program
// on the node's real PTY switches modes on before a window attaches, and logs every byte it receives:
//   D1  after a reload and after Home and back, a click, a drag and the wheel reach the program as mouse reports;
//   D2  after a reload, the wheel on the program's alternate screen sends arrow keys;
//   D3  after a reconnect (hub restarted) in which the program left the alternate screen and switched the mouse off, the wheel and
//       a click send nothing to it;
//   D5  after a reload, focus reports (1004) arrive;
//   and in `agora tui` (Linux, a pty): a late attach to a program with the mouse on gets the click and the wheel (D1), and to one on
//   the alternate screen gets arrow keys for the wheel (D2).
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-terminal-repaint.mjs [chromium|firefox|webkit]
//   (AGORA_BIN=<another agora> runs the same checks against that build: an older one shows the defects.)
//
// Needs git, sh and python3 on the node's machine (Linux or macOS). Uses a throw-away folder; changes nothing else.
import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { startStack } from './browser-harness.mjs';

const here = resolve(fileURLToPath(new URL('./terminal-repaint', import.meta.url)));
const engine = process.argv[2] ?? 'chromium';
const sleep = ms => new Promise(r => setTimeout(r, ms));
// Polls `ok` until it returns something truthy, or throws `what` after `ms`: every wait in this script is for a condition, never a
// fixed pause (a loaded CI runner only makes it slower).
async function until(what, ok, ms = 20000) {
  for (const end = Date.now() + ms; ;) {
    const v = await ok();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(50);
  }
}
const hex = s => Buffer.from(s, 'latin1').toString('hex');
const show = h => Buffer.from(h, 'hex').toString('latin1').replace(/[\x00-\x1f\x7f-\xff]/g, c => (c === '\x1b' ? '\\e' : `\\x${c.charCodeAt(0).toString(16).padStart(2, '0')}`)) || '(nothing)';
const sgr = (b, c, r, release = false) => hex(`\x1b[<${b};${c + 1};${r + 1}${release ? 'm' : 'M'}`);

let failed = false;
const check = (what, got, ok) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}${ok ? '' : `: got ${show(got)}`}`); if (!ok) failed = true; };
const exact = (what, got, want) => check(what, got, got === want);

const visible = page => page.evaluate(() => [...document.querySelectorAll('.terminal')].find(e => e.getBoundingClientRect().width > 0)?.querySelector('.xterm-rows')?.innerText ?? '');
// The visible terminal's cell grid, once it is laid out (its size known and its screen measured).
let lastGeometry;
const geometry = page => until('the terminal to be laid out', async () => {
  const g = await measureGeometry(page);
  lastGeometry = g;
  return g && Number.isFinite(g.cw) && Number.isFinite(g.ch) && g.cw > 0 && g.ch > 0 ? g : null;
});
async function measureGeometry(page) {
  return page.evaluate(() => {
    // The terminal's host element says the size the terminal is shown at (TerminalView.tsx).
    const t = [...document.querySelectorAll('[data-cols]')].find(e => e.getBoundingClientRect().width > 0);
    const screen = t?.querySelector('.xterm-screen');
    if (!screen) return null;
    const s = screen.getBoundingClientRect();
    const cols = Number(t.dataset.cols), rows = Number(t.dataset.rows);
    return { x: s.x, y: s.y, cw: s.width / cols, ch: s.height / rows };
  });
}
const at = (g, c, r) => ({ x: g.x + (c + 0.5) * g.cw, y: g.y + (r + 0.5) * g.ch });

let stack;
try {
  // AGORA_BIN runs the checks against another build (an older one shows the defects).
  stack = await startStack({ engine, viewport: { width: 1280, height: 800 }, binary: process.env.AGORA_BIN });
  const { page, start, work, origin } = stack;
  await start('Shell');
  await page.waitForSelector('.xterm-helper-textarea', { state: 'attached', timeout: 15000 });
  await page.waitForFunction(() => /\$\s*$/.test((document.querySelector('.xterm-rows')?.innerText ?? '').trim()), null, { timeout: 30000 });
  await page.focus('.xterm-helper-textarea');
  // Git for Windows' sh drops keys while it starts (see e2e-browser.mjs); the same Enter-until-answered here.
  for (let tries = 1; tries <= 5; tries++) {
    const before = (await visible(page)).split('$').length;
    await page.keyboard.press('Enter');
    if (await page.waitForFunction(n => (document.querySelector('.xterm-rows')?.innerText ?? '').split('$').length > n, before, { timeout: 3000 }).then(() => true, () => false)) break;
  }
  const agentUrl = page.url();
  let n = 0;
  // Starts the probe in the shell from the window; returns its log and control file.
  const shows = text => until(`the terminal to show ${text}`, async () => (await visible(page)).includes(text), 60000);
  const probe = async args => {
    n += 1;
    const log = join(work, `p${n}.log`), ctl = join(work, `p${n}.ctl`);
    await page.focus('.xterm-helper-textarea');
    await page.keyboard.type(`python3 ${join(here, 'probe.py')} ${log} --ctl ${ctl} ${args}\n`, { delay: 2 });
    const lines = () => (existsSync(log) ? readFileSync(log, 'utf8').trim().split('\n').filter(Boolean).map(l => JSON.parse(l)) : []);
    await until('the probe to start', () => lines().some(l => l.event === 'ready'));
    await shows('PROBE READY');
    const input = from => lines().slice(from).filter(l => l.hex).map(l => l.hex).join('');
    // A key typed into the terminal after an action arrives after everything the action sent (input keeps its order), so the
    // bytes before it are exactly the action's, with no guess at how long to wait. `Y` marks the start, `Z` the end.
    // Types `key` and waits until the probe received it after log line `from`; returns the bytes before it and the log length then.
    const mark = async (key, from) => {
      await page.keyboard.type(key);
      const k = hex(key);
      return until(`the probe to receive ${key}`, () => {
        const h = input(from);
        for (let i = 0; i < h.length; i += 2) if (h.startsWith(k, i)) return { before: h.slice(0, i), at: lines().length };
        return null;
      });
    };
    return {
      log, ctl,
      measure: async action => {
        // The terminal has the keyboard before the start mark, so focusing it reports nothing inside the measurement.
        await page.focus('.xterm-helper-textarea');
        const { at } = await mark('Y', lines().length);
        await action();
        return (await mark('Z', at)).before;
      },
      send: async line => {
        const sent = lines().filter(l => l.event === 'ctl').length;
        appendFileSync(ctl, `${line}\n`);
        await until('the probe to write the line', () => lines().filter(l => l.event === 'ctl').length > sent);
      },
      quit: async () => { appendFileSync(ctl, 'quit\n'); await until('the probe to exit', () => lines().some(l => l.event === 'exit')); },
    };
  };
  const reattach = async () => { await page.reload(); await page.waitForSelector('.xterm-screen', { timeout: 20000 }); await shows('PROBE READY'); };
  let g;
  const click = (c, r) => async () => { const q = at(g, c, r); await page.mouse.move(q.x, q.y); await page.mouse.down(); await page.mouse.up(); };
  const wheel = (dy, c = 7, r = 4) => async () => { const q = at(g, c, r); await page.mouse.move(q.x, q.y); await page.mouse.wheel(0, dy); };

  // D1, D5: mouse and focus modes switched on before the window attached.
  const p1 = await probe('--modes 1000,1002,1006,1004');
  await reattach();
  g = await geometry(page);
  // The click also focuses the terminal, which the program hears first (1004 is on): ESC [ I.
  const ca = await p1.measure(click(5, 3));
  check('D1 after a reload: a click is reported', ca, ca.replace(/^1b5b49/, '') === sgr(0, 5, 3) + sgr(0, 5, 3, true));
  const drag = await p1.measure(async () => { const a = at(g, 5, 3), b = at(g, 9, 3); await page.mouse.move(a.x, a.y); await page.mouse.down(); await page.mouse.move(b.x, b.y, { steps: 4 }); await page.mouse.up(); });
  check('D1 after a reload: a drag is reported (1002)', drag, drag.startsWith(sgr(0, 5, 3)) && drag.includes(sgr(32, 9, 3)) && drag.endsWith(sgr(0, 9, 3, true)));
  exact('D1 after a reload: the wheel is reported', await p1.measure(wheel(-100)), sgr(64, 7, 4));
  const focus = await p1.measure(async () => { await page.evaluate(() => document.activeElement?.blur()); await until('the terminal to lose the keyboard', () => page.evaluate(() => !document.activeElement?.classList.contains('xterm-helper-textarea'))); await page.focus('.xterm-helper-textarea'); });
  exact('D5 after a reload: blur and focus are reported', focus, hex('\x1b[O\x1b[I'));
  await page.goto(`${origin}/#/`);
  await until('the terminal to close', async () => !(await page.$('.xterm-screen')));
  await page.goto(agentUrl);
  await page.waitForSelector('.xterm-screen', { timeout: 20000 });
  await shows('PROBE READY');
  g = await geometry(page);
  // The click also focuses the terminal, which the program hears first (1004 is on): ESC [ I.
  const cH = await p1.measure(click(5, 3));
  check('D1 after Home and back: a click is reported', cH, cH.replace(/^1b5b49/, '') === sgr(0, 5, 3) + sgr(0, 5, 3, true));
  await p1.quit();

  // D2: a full-screen program with application cursor keys, no mouse.
  const p2 = await probe('--alt --modes 1');
  await reattach();
  g = await geometry(page);
  exact('D2 after a reload: the wheel on the alternate screen sends arrow keys', await p2.measure(wheel(-100)), hex('\x1bOA'.repeat(3)));
  await p2.quit();

  // D3: the program leaves the alternate screen and switches the mouse off while the window is away.
  const p3 = await probe('--alt --modes 1000,1006');
  g = await geometry(page);
  check('control: the wheel is reported while the window watches', '', (await p3.measure(wheel(-100))).startsWith(hex('\x1b[<64;')));
  await stack.stopHub();
  await p3.send('\\e[?1000l\\e[?1006l\\e[?1049l\\r\\nBACK ON THE MAIN SCREEN, MOUSE OFF\\r\\n');
  stack.startHub();
  const back = await page.waitForFunction(() => (document.querySelector('.xterm-rows')?.innerText ?? '').includes('BACK ON THE MAIN SCREEN'), null, { timeout: 60000 }).then(() => true, () => false);
  check('D3 the window reconnected and shows the program\'s screen', '', back);
  g = await geometry(page);
  exact('D3 after the reconnect: the wheel sends nothing', await p3.measure(wheel(-100)), '');
  exact('D3 after the reconnect: a click sends nothing', await p3.measure(click(5, 3)), '');
  await p3.quit();

  // The terminal client (Linux: the driver needs a pty). The window leaves the terminal so only the client watches it.
  if (process.platform === 'linux') {
    for (const [args, mode] of [['--modes 1000,1006', 'mouse'], ['--alt', 'alt']]) {
      await page.goto(agentUrl);
      await page.waitForSelector('.xterm-helper-textarea', { state: 'attached', timeout: 20000 });
      const p = await probe(args);
      await page.goto(`${origin}/#/`);
      await until('the terminal to close', async () => !(await page.$('.xterm-screen')));
      const out = execFileSync('python3', [join(here, 'tui-driver.py'), stack.bin, stack.hubDir, p.log, p.ctl, mode], { env: stack.env, timeout: 120000 }).toString();
      const got = Object.fromEntries(out.trim().split('\n').filter(l => l.startsWith('{')).map(l => JSON.parse(l)).map(r => [r.name, r.hex]));
      console.log(`     terminal client got: ${Object.entries(got).map(([k, v]) => `${k} ${show(v)}`).join(', ')}`);
      if (mode === 'mouse') {
        check('D1 terminal client, late attach: the first click is reported', got.click ?? '', (got.click ?? '').startsWith(hex('\x1b[<0;')));
        check('D1 terminal client, late attach: the wheel is reported', got.wheel ?? '', (got.wheel ?? '').startsWith(hex('\x1b[<64;')));
      } else {
        exact('D2 terminal client, late attach: the wheel on the alternate screen sends arrow keys', got.wheel ?? '', hex('\x1b[A'.repeat(3)));
      }
      await p.quit();
    }
  }
  check('no page errors', stack.errors.join('; '), stack.errors.length === 0);
} catch (e) {
  console.log(`FAIL ${e.stack}${lastGeometry !== undefined ? ` (last geometry ${JSON.stringify(lastGeometry)})` : ''}`);
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
