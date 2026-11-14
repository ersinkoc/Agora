// ADR-0070 acceptance, in a real browser: links that go silent without ending, and a hub that is killed and started again.
//  1. A frozen machine: its node process is stopped (SIGSTOP), so its connection stays open and says nothing, as a laptop that
//     slept looks to the hub. The window shows it offline with "last seen" within the hub's silence deadline (25 s). When it runs
//     again (SIGCONT) it is online again by itself, and its agent is still running (nothing was reset).
//  2. A frozen hub (SIGSTOP past the deadline): the window says "Hub unreachable"; after SIGCONT the window and the machine are back.
//  3. A killed hub (SIGKILL), started again at once on the same data and port: the window and the machine are back.
// The times are printed; the checks have generous limits (the deadline plus a backoff). Uses POSIX signals, so it runs on Linux
// and macOS; on Windows the freezes are skipped and only the restart runs. Only a shell agent is used. See browser-harness.mjs for
// prerequisites (BINARY overrides target/debug/agora).
import { execFileSync, spawn } from 'node:child_process';
import { join } from 'node:path';
import { root, startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const posix = process.platform !== 'win32';
// The hub's silence deadline (`NODE_SILENCE`) and the window's ping deadline (15 s + 10 s), with room for a backoff.
const NOTICE_MS = 45_000;

// The pid of the one process whose command line contains `pattern` (a unique temporary path is part of it).
function pidOf(pattern) {
  const out = execFileSync('pgrep', ['-f', '--', pattern]).toString().trim().split('\n').filter(Boolean);
  if (out.length !== 1) throw new Error(`expected one process for ${pattern}, found ${out.length}`);
  return Number(out[0]);
}

let stack;
let hub2;
try {
  const binary = process.env.BINARY;
  stack = await startStack({ hostname: 'laptop-host', ...(binary ? { binary } : {}) });
  const { page, run, proj, hubDir, origin, errors } = stack;
  const bin = binary ?? join(root, 'target/debug/agora');
  const port = new URL(origin).port;
  page.setDefaultTimeout(15000);
  const machine = 'section[data-machine-id="laptop"]';
  const online = state => page.waitForSelector(`${machine}[data-online="${state}"]`, { timeout: 120_000 }).then(() => true, () => false);
  // How long until `work` resolves true, or 0 when it did not.
  const timed = async work => { const t = Date.now(); return (await work) ? Date.now() - t : 0; };

  const agent = run(['agent', 'start', 'shell', '--cwd', proj]).match(/agent-\d+/)?.[0];
  check(!!agent, `an agent started on the laptop (${agent})`);
  const running = () => run(['agent', 'list']).split('\n').some(line => line.startsWith(`${agent}\t`) && line.includes('running'));
  await page.goto(`${origin}/#/`);
  check(await online('true'), 'the window shows the laptop online');

  if (posix) {
    // 1. A frozen machine.
    const node = pidOf(`node run --project ${proj}`);
    process.kill(node, 'SIGSTOP');
    const offMs = await timed(page.waitForSelector(`${machine}[data-online="false"]`, { timeout: NOTICE_MS }).then(() => true, () => false));
    check(offMs > 0, `the window shows the frozen laptop offline (after ${offMs} ms)`);
    const seen = await page.locator(`${machine} [data-last-seen]`).first().innerText().catch(() => '');
    check(/last seen/.test(seen), `with when it was last seen ("${seen}")`);
    await wait(5000);
    process.kill(node, 'SIGCONT');
    const backMs = await timed(online('true'));
    check(backMs > 0, `the laptop is online again by itself after it runs again (after ${backMs} ms)`);
    check(running(), 'its agent is still running');
    check(await page.locator(`${machine} button[aria-pressed]`).count() >= 1, 'and the window still lists it');

    // 2. A frozen hub.
    const hub = pidOf(`hub run --listen 127.0.0.1:${port} --hub-dir ${hubDir}`);
    process.kill(hub, 'SIGSTOP');
    const lostMs = await timed(page.getByText('Hub unreachable').first().waitFor({ timeout: NOTICE_MS }).then(() => true, () => false));
    check(lostMs > 0, `the window notices the frozen hub (after ${lostMs} ms)`);
    await wait(Math.max(0, 40_000 - lostMs));
    process.kill(hub, 'SIGCONT');
    const t = Date.now();
    const windowMs = await timed(page.getByText('Hub unreachable').first().waitFor({ state: 'detached', timeout: 60_000 }).then(() => true, () => false));
    const nodeMs = (await online('true')) ? Date.now() - t : 0;
    check(windowMs > 0 && nodeMs > 0, `after the hub runs again the window is back in ${windowMs} ms and the laptop online in ${nodeMs} ms`);
    check(running(), 'the laptop\'s agent is still running');
  } else {
    console.log('skip the freezes (no SIGSTOP on Windows)');
  }

  // 3. A killed hub, started again at once on the same data and port.
  const hub = pidOf(`hub run --listen 127.0.0.1:${port} --hub-dir ${hubDir}`);
  process.kill(hub, 'SIGKILL');
  await page.getByText('Hub unreachable').first().waitFor({ timeout: NOTICE_MS }).catch(() => {});
  hub2 = spawn(bin, ['hub', 'run', '--listen', `127.0.0.1:${port}`, '--hub-dir', hubDir, '--web', join(root, 'apps/web/dist')], { env: stack.env, stdio: 'ignore' });
  const up = Date.now();
  const windowMs = await timed(page.getByText('Hub unreachable').first().waitFor({ state: 'detached', timeout: 60_000 }).then(() => true, () => false));
  const nodeMs = (await online('true')) ? Date.now() - up : 0;
  check(windowMs > 0 && nodeMs > 0, `after a hub restart the window is back in ${windowMs} ms and the laptop online in ${nodeMs} ms`);
  check(running(), 'the laptop\'s agent is still running');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  check(false, e.message.split('\n')[0]);
} finally {
  hub2?.kill();
  await stack?.close();
}
process.exit(failed ? 1 : 0);
