// The agent tree with a large fleet (docs/design/performance.md: "500 agents across 50 projects, scroll at 60 fps"; picking an agent
// within the 50 ms navigation budget). Starts a hub on a throw-away home, seeds it with scripts/perf-seed-agents.py (10 machines x
// 5 projects x 10 agents by default; AGORA_PERF_FLEET="nodes projects agents" to change), signs in with headless Chromium and prints:
// the time until every agent row is drawn, DOM size and heap, frame gaps while scrolling the whole tree, the time from a click on an
// agent to the next frame, and a full re-render of the tree (a filter that keeps every row). Nothing else on the machine is touched.
//
//   cargo build -p agora && (cd apps/web && npm ci && npm run gen && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/perf-tree.mjs
import { execFileSync, spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const bin = join(root, 'target/debug/agora');
const home = mkdtempSync(join(tmpdir(), 'agora-perf-tree-'));
const hubDir = join(home, 'hub');
const env = { ...process.env, AGORA_HOME: home, AGORA_LANG: 'en' };
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const fleet = (process.env.AGORA_PERF_FLEET ?? '10 5 10').split(/\s+/);
const total = fleet.reduce((a, b) => a * Number(b), 1);
const hub = () => spawn(bin, ['hub', 'run', '--listen', '127.0.0.1:0', '--hub-dir', hubDir, '--web', join(root, 'apps/web/dist')], { env, stdio: 'ignore' });
const waitFor = async (ok) => { for (let i = 0; i < 100; i++) { try { if (ok()) return; } catch {} await new Promise((r) => setTimeout(r, 100)); } throw new Error('timed out'); };

let h = hub();
let browser;
try {
  // The first start makes the database; the fleet is written while the hub is stopped, then it starts on it.
  await waitFor(() => readFileSync(join(hubDir, 'listen'), 'utf8'));
  h.kill(); await new Promise((r) => h.once('exit', r));
  execFileSync('python3', ['-I', join(root, 'scripts/perf-seed-agents.py'), join(hubDir, 'hub.db'), ...fleet], { stdio: 'ignore' });
  rmSync(join(hubDir, 'listen'));
  h = hub();
  await waitFor(() => readFileSync(join(hubDir, 'listen'), 'utf8'));
  const addr = readFileSync(join(hubDir, 'listen'), 'utf8').trim();
  const code = execFileSync(bin, ['hub', 'login', '--hub-dir', hubDir], { env }).toString().match(/[A-Z0-9]{4}-[A-Z0-9]{4}/)[0];

  browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
  const page = await (await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 900 } })).newPage();
  const rows = 'aside nav li button.ide-row';
  await page.goto(`https://${addr}/`);
  await page.fill('input', code);
  await page.click('button:has-text("Sign in")');
  await page.waitForFunction(([s, n]) => document.querySelectorAll(s).length >= n, [rows, total], { timeout: 30_000 });
  const t0 = Date.now();
  await page.reload();
  await page.waitForFunction(([s, n]) => document.querySelectorAll(s).length >= n, [rows, total], { timeout: 30_000 });
  const firstMs = Date.now() - t0;
  await page.waitForTimeout(500);
  const result = await page.evaluate(async (s) => {
    const pct = (xs, q) => { const v = [...xs].sort((a, b) => a - b); return Number(v[Math.floor(q * (v.length - 1))].toFixed(1)); };
    const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
    const nav = document.querySelector('aside nav');
    const size = { rows: nav.querySelectorAll('li button.ide-row').length, treeNodes: nav.querySelectorAll('*').length,
      pageNodes: document.querySelectorAll('*').length, heapMB: performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1e6) : null };
    const long = [];
    new PerformanceObserver((l) => l.getEntries().forEach((e) => long.push(e.duration))).observe({ type: 'longtask' });
    const gaps = []; let last = performance.now();
    for (let i = 1; i <= 120; i++) {
      nav.scrollTop = i * (nav.scrollHeight - nav.clientHeight) / 120;
      await new Promise((r) => requestAnimationFrame(r));
      const now = performance.now(); gaps.push(now - last); last = now;
    }
    nav.scrollTop = 0;
    const all = [...document.querySelectorAll(s)];
    const pick = [];
    for (let i = 1; i <= 12; i++) {
      const t = performance.now(); all[Math.floor(i * all.length / 13)].click(); await frame(); pick.push(performance.now() - t);
      await new Promise((r) => setTimeout(r, 150));
    }
    const input = document.querySelector('aside input[type=search]');
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    const render = [];
    for (let i = 0; i < 20; i++) {
      const t = performance.now(); set.call(input, i % 2 ? 'a' : 'e'); input.dispatchEvent(new Event('input', { bubbles: true })); await frame();
      render.push(performance.now() - t); await new Promise((r) => setTimeout(r, 50));
    }
    return { ...size, scrollFrameMs: { p50: pct(gaps, 0.5), p95: pct(gaps, 0.95), max: pct(gaps, 1) }, longTasksWhileScrolling: long.length,
      pickToFrameMs: { p50: pct(pick, 0.5), p95: pct(pick, 0.95) }, fullRerenderMs: { p50: pct(render, 0.5), p95: pct(render, 0.95) } };
  }, rows);
  console.log(JSON.stringify({ fleet: fleet.join('x'), agents: total, firstLoadMs: firstMs, ...result }, null, 1));
} finally {
  await browser?.close();
  h.kill();
  rmSync(home, { recursive: true, force: true });
}
