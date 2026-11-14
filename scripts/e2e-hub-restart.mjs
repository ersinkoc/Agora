// The hub is killed and started again, many times, under two machines and a signed-in window. After each start an agent is
// started on each machine at once, and the script measures how long until the open window shows it: that is how long a new
// event takes to reach a person after a hub restart. The hub stays down for a different time in each round (HUB_DOWN_MS, a
// comma-separated list cycled through; default 0..20 s), since a node's redial schedule depends on how long it has been failing.
// Each round must be under LIMIT_MS (default 3000). ROUNDS sets how many (default 8). The time until the window no longer says
// "Hub unreachable" or "Connecting" is printed too. DIAG=1 (Linux, needs `ss`) also prints when each process reconnected. Only
// shells are used; see browser-harness.mjs for prerequisites.
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const rounds = Number(process.env.ROUNDS ?? 8);
const limit = Number(process.env.LIMIT_MS ?? 3000);
const downs = (process.env.HUB_DOWN_MS ?? '0,1000,3000,6000,10000,16000,20000,2000').split(',').map(Number);

let stack;
try {
  stack = await startStack({ hostname: 'laptop-host' });
  const { page, run, proj, work, env, origin } = stack;
  const clone = join(work, 'desktop-proj');
  execFileSync('git', ['clone', '-q', proj, clone], { env });
  const desktop = await stack.addNode({ id: 'desktop', hostname: 'desktop-host', project: clone });
  await page.goto(`${origin}/#/`);
  await page.waitForFunction(() => document.querySelectorAll('aside section[data-machine-id]').length === 2, null, { timeout: 30000 });
  // How many running agents the window lists for a machine (the rows under its folders, not the interrupted ones).
  const count = machine => page.evaluate(m => [...document.querySelectorAll(`aside section[data-machine-id="${m}"] li > button[aria-pressed]`)]
    .filter(b => !b.closest('[role=group]')).length, machine);
  const shown = (machine, n) => page.waitForFunction(([m, k]) => [...document.querySelectorAll(`aside section[data-machine-id="${m}"] li > button[aria-pressed]`)]
    .filter(b => !b.closest('[role=group]')).length >= k, [machine, n], { timeout: 60000, polling: 50 }).then(() => true, () => false);
  const machines = [
    { id: 'laptop', start: () => run(['agent', 'start', 'shell', '--cwd', proj]) },
    { id: 'desktop', start: () => desktop.run(['agent', 'start', 'shell', '--cwd', clone]) },
  ];
  const times = { laptop: [], desktop: [] };
  for (let r = 0; r < rounds; r++) {
    const down = downs[r % downs.length];
    const before = await Promise.all(machines.map(m => count(m.id)));
    await stack.stopHub();
    await wait(down);
    stack.startHub();
    const up = Date.now();
    // DIAG=1 (Linux): when each client process (by pid) first has an established connection to the hub after it started.
    const firsts = new Map();
    const diag = process.env.DIAG ? setInterval(() => {
      let out = '';
      try { out = execFileSync('ss', ['-tnpH', 'state', 'established', `( dport = :${process.env.AGORA_TEST_PORT ?? 17443} )`]).toString(); } catch { return; }
      for (const m of out.matchAll(/\(\("([^"]+)",pid=(\d+)/g)) if (!firsts.has(m[2])) firsts.set(m[2], `${m[1]}:${m[2]}@${Date.now() - up}`);
    }, 20) : undefined;
    // The window's own link, timed on its own: when it no longer says the hub is unreachable or that it is connecting.
    const banner = page.waitForFunction(() => !/Hub unreachable|Connecting to your hub/.test(document.body.innerText), null, { timeout: 60000, polling: 50 })
      .then(() => Date.now() - up, () => -1);
    // Both agents start at once; each is timed from the hub's start to when the window shows it.
    const seen = await Promise.all(machines.map(async (m, i) => {
      if (!m.start().match(/agent-\d+/)) return -1;
      return (await shown(m.id, before[i] + 1)) ? Date.now() - up : -1;
    }));
    machines.forEach((m, i) => times[m.id].push(seen[i]));
    const live = await banner;
    if (diag) { clearInterval(diag); console.log(`     connected: ${[...firsts.values()].join(' ')}`); }
    console.log(`     round ${r + 1}: down ${down} ms; after the hub started: the window live in ${live} ms, the new agent shown: laptop ${seen[0]} ms, desktop ${seen[1]} ms`);
    for (const [i, m] of machines.entries()) check(seen[i] >= 0 && seen[i] <= limit, `round ${r + 1}: the ${m.id}'s new agent is shown within ${limit} ms (${seen[i]} ms)`);
    // The connection the hub closed must not count as a suspension or a silence in the next round.
    await wait(1000);
  }
  for (const [m, t] of Object.entries(times)) {
    const ok = t.filter(x => x >= 0).sort((a, b) => a - b);
    console.log(`     ${m}: ${t.join(' ')} ms; median ${ok[Math.floor(ok.length / 2)]} ms, worst ${ok[ok.length - 1]} ms`);
  }
  check(stack.errors.length === 0, `no page errors${stack.errors.length ? `: ${stack.errors[0]}` : ''}`);
} catch (e) {
  check(false, e.message.split('\n')[0]);
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
