// The agent tree with a large fleet (docs/design/performance.md: "500 agents across 50 projects, scroll at 60 fps"; picking an agent
// within the 50 ms navigation budget). Starts a hub on a throw-away home, seeds it with scripts/perf-seed-agents.py (10 machines x
// 5 projects x 10 agents by default; AGORA_PERF_FLEET="nodes projects agents" to change), signs in with headless Chromium and prints:
// warm reload until the fleet is loaded and near-viewport rows exist, DOM size and heap, scroll frame gaps, programmatic
// row click to frame, and local tree-filter change to frame. Long tasks are counted separately by phase start time. The seeded
// fleet has no live node connections or terminal streams; this does not measure live view-to-model updates.
//
//   cargo build -p agora && (cd apps/web && npm ci && npm run gen && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/perf-tree.mjs
// Existing build artifacts can be supplied with AGORA_PERF_BIN and AGORA_PERF_WEB_DIR. PYTHON selects the Python executable
// (default python3); on Windows also set CHROME and AGORA_PERF_BIN to the installed browser and built agora.exe.
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { createRequire } from 'node:module';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const bin = process.env.AGORA_PERF_BIN ?? join(root, 'target/debug/agora');
const web = process.env.AGORA_PERF_WEB_DIR ?? join(root, 'apps/web/dist');
const screenshots = process.env.AGORA_SCREENSHOTS;
const home = mkdtempSync(join(tmpdir(), 'agora-perf-tree-'));
const hubDir = join(home, 'hub');
const env = { ...process.env, AGORA_HOME: home, AGORA_LANG: 'en' };
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const fleet = (process.env.AGORA_PERF_FLEET ?? '10 5 10').split(/\s+/);
const total = fleet.reduce((a, b) => a * Number(b), 1);
const hub = () => spawn(bin, ['hub', 'run', '--listen', '127.0.0.1:0', '--hub-dir', hubDir, '--web', web], { env, stdio: 'ignore' });
const stopHub = async (child) => {
  if (child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit');
    child.kill();
    await exited;
  }
};
const waitFor = async (ok) => { for (let i = 0; i < 100; i++) { try { if (ok()) return; } catch {} await new Promise((r) => setTimeout(r, 100)); } throw new Error('timed out'); };

let h = hub();
let browser;
try {
  // The first start makes the database; the fleet is written while the hub is stopped, then it starts on it.
  await waitFor(() => readFileSync(join(hubDir, 'listen'), 'utf8'));
  await stopHub(h);
  execFileSync(process.env.PYTHON ?? 'python3', ['-I', join(root, 'scripts/perf-seed-agents.py'), join(hubDir, 'hub.db'), ...fleet], { stdio: 'ignore' });
  rmSync(join(hubDir, 'listen'));
  h = hub();
  await waitFor(() => readFileSync(join(hubDir, 'listen'), 'utf8'));
  const addr = readFileSync(join(hubDir, 'listen'), 'utf8').trim();
  const code = execFileSync(bin, ['hub', 'login', '--hub-dir', hubDir], { env }).toString().match(/[A-Z0-9]{4}-[A-Z0-9]{4}/)[0];

  browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
  const page = await (await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 900 } })).newPage();
  const rows = 'aside nav li button.ide-row';
  const waitForRows = () => page.waitForFunction(([selector, count]) => {
    const mounted = document.querySelectorAll(selector).length;
    const fleetSize = document.querySelector('aside nav')?.dataset.agentCount;
    return fleetSize === undefined ? mounted >= count : Number(fleetSize) >= count && mounted > 0;
  }, [rows, total], { timeout: 30_000 });
  await page.goto(`https://${addr}/`);
  await page.fill('input', code);
  await page.click('button:has-text("Sign in")');
  await waitForRows();
  const t0 = Date.now();
  await page.reload();
  await waitForRows();
  const domRowsReadyMs = Date.now() - t0;
  await page.waitForTimeout(500);
  const result = await page.evaluate(async (s) => {
    const pct = (xs, q) => { const v = [...xs].sort((a, b) => a - b); return Number(v[Math.floor(q * (v.length - 1))].toFixed(1)); };
    const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
    const nav = document.querySelector('aside nav');
    const size = { rows: nav.querySelectorAll('li button.ide-row').length, treeNodes: nav.querySelectorAll('*').length,
      pageNodes: document.querySelectorAll('*').length, heapMB: performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1e6) : null };
    const visible = () => {
      const { top, bottom } = nav.getBoundingClientRect();
      return [...document.querySelectorAll(s)].filter(row => {
        const rect = row.getBoundingClientRect();
        return rect.bottom > top && rect.top < bottom;
      });
    };
    const firstRow = visible()[0]?.dataset.agentRow;
    const long = [];
    const observer = new PerformanceObserver((l) => long.push(...l.getEntries()));
    observer.observe({ type: 'longtask' });
    const phases = { scroll: { start: performance.now(), end: 0 }, pick: { start: 0, end: 0 }, filter: { start: 0, end: 0 } };
    const gaps = []; let last = performance.now();
    for (let i = 1; i <= 120; i++) {
      nav.scrollTop = i * (nav.scrollHeight - nav.clientHeight) / 120;
      await new Promise((r) => requestAnimationFrame(r));
      const now = performance.now(); gaps.push(now - last); last = now;
    }
    nav.scrollTop = nav.scrollHeight;
    await frame();
    nav.scrollTop = nav.scrollHeight;
    await frame();
    const lastRow = visible().at(-1)?.dataset.agentRow;
    if (!firstRow || !lastRow || (nav.scrollHeight > nav.clientHeight * 2 && firstRow === lastRow)) {
      throw new Error('Scroll did not reveal the bottom of the fleet');
    }
    phases.scroll.end = performance.now();
    const pick = [];
    phases.pick.start = performance.now();
    for (let i = 1; i <= 12; i++) {
      nav.scrollTop = i * (nav.scrollHeight - nav.clientHeight) / 13;
      await frame();
      await frame();
      const inView = visible();
      const row = inView[Math.floor(inView.length / 2)];
      if (!row) throw new Error(`No agent visible at scroll sample ${i}`);
      const t = performance.now();
      row.click();
      await frame();
      pick.push(performance.now() - t);
      await new Promise((r) => setTimeout(r, 150));
    }
    phases.pick.end = performance.now();
    nav.scrollTop = 0;
    await frame();
    const input = document.querySelector('aside input[type=search]');
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    const filter = [];
    phases.filter.start = performance.now();
    for (let i = 0; i < 20; i++) {
      const t = performance.now(); set.call(input, i % 2 ? 'a' : 'e'); input.dispatchEvent(new Event('input', { bubbles: true })); await frame();
      filter.push(performance.now() - t); await new Promise((r) => setTimeout(r, 50));
    }
    phases.filter.end = performance.now();
    // Finish the current task before draining observer delivery; takeRecords includes entries not yet sent to its callback.
    await new Promise((r) => setTimeout(r, 0));
    long.push(...observer.takeRecords());
    observer.disconnect();
    const counts = Object.fromEntries(Object.entries(phases).map(([name, phase]) =>
      [name, long.filter((e) => e.startTime >= phase.start && e.startTime < phase.end).length]));
    return { ...size, scrollFrameMs: { p50: pct(gaps, 0.5), p95: pct(gaps, 0.95), max: pct(gaps, 1) },
      longTasksWhileScrolling: counts.scroll, longTasksWhilePicking: counts.pick, longTasksWhileFiltering: counts.filter,
      longTasksOutsidePhases: long.length - counts.scroll - counts.pick - counts.filter, longTasksTotal: long.length,
      pickToFrameMs: { p50: pct(pick, 0.5), p95: pct(pick, 0.95) }, filterToFrameMs: { p50: pct(filter, 0.5), p95: pct(filter, 0.95) } };
  }, rows);
  const explorerNav = page.locator('aside nav').first();
  const agents = explorerNav.locator('[data-agent-row]');
  await page.locator('aside input[type=search]').fill('');
  await explorerNav.evaluate(nav => { nav.scrollTop = 0; });
  await waitForRows();
  // The keyboard must continue in reading order as scrolling reveals the next set of rows.
  const visited = new Set();
  if (total >= 120) {
    const first = agents.first();
    const firstId = await first.getAttribute('data-agent-row');
    await first.focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(id => document.querySelector('aside nav [aria-current="true"]')?.getAttribute('data-agent-row') === id, firstId);

    for (let i = 0; i < 40; i++) {
      await page.keyboard.press('Tab');
      const id = await page.evaluate(() => document.activeElement?.getAttribute('data-agent-row'));
      if (id) visited.add(id);
    }
    if (visited.size < 12) throw new Error(`Keyboard visited only ${visited.size} rows`);
  }
  if (screenshots) {
    const explorer = page.getByRole('complementary', { name: 'Agents', exact: true });
    await explorerNav.evaluate(nav => {
      document.activeElement?.blur();
      nav.scrollTop = 0;
    });
    for (const colorScheme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme });
      await explorer.screenshot({ path: join(screenshots, `explorer-windowing-${colorScheme}.png`) });
    }
  }
  console.log(JSON.stringify({ fleet: fleet.join('x'), agents: total, fixture: 'static-fleet-no-live-nodes-or-terminal-streams', domRowsReadyMs, keyboardRowsVisited: visited.size, ...result }, null, 1));
} finally {
  try {
    await browser?.close();
  } finally {
    await stopHub(h);
    rmSync(home, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
