// End-to-end check in a real browser: a hub and a node on this machine, sign in with a code, start a shell agent and type into
// its terminal, start a chat agent, send it a request, and answer its permission request from the window.
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-browser.mjs
//
// Needs git, sh and a Chromium (CHROME=<path>, default: Playwright's cache). Uses a throw-away folder; changes nothing else.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const root = resolve(new URL('..', import.meta.url).pathname);
const bin = join(root, 'target/debug/agora');
const fake = join(root, 'target/debug/agora-fake-agent');
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;

const work = mkdtempSync(join(tmpdir(), 'agt-')); // short: Unix socket paths are limited to about 100 bytes
const home = join(work, 'h');
const hubDir = join(work, 'hub');
const proj = join(work, 'proj');
const env = { ...process.env, AGORA_HOME: home, AGORA_LANG: 'en', GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@x', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@x' };
const run = (args) => execFileSync(bin, args, { env }).toString();
const children = [];
let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };

try {
  mkdirSync(join(home, 'manifests'), { recursive: true });
  writeFileSync(join(home, 'manifests/shell.toml'), 'kind = "shell"\ndisplay_name = "Shell"\nversion = "1"\nmin_engine = 1\n[process]\nlaunch = ["sh"]\n');
  writeFileSync(join(home, 'manifests/fake.toml'), `kind = "fake-adapter"\ndisplay_name = "Fake chat agent"\nversion = "1"\nmin_engine = 1\n[adapter]\ncommand = [${JSON.stringify(fake)}, "--adapter"]\n`);
  mkdirSync(proj);
  for (const a of [['init', '-q', '-b', 'main'], ['commit', '-q', '--allow-empty', '-m', 'init']]) execFileSync('git', ['-C', proj, ...a], { env });

  const hub = spawn(bin, ['hub', 'run', '--listen', '127.0.0.1:7443', '--hub-dir', hubDir, '--web', join(root, 'apps/web/dist')], { env, stdio: 'ignore' });
  children.push(hub);
  await new Promise((r) => setTimeout(r, 2500));
  const code = run(['hub', 'code', '--hub-dir', hubDir]).match(/[A-Z0-9]{4}-[A-Z0-9]{4}/)[0];
  run(['node', 'enroll', '--hub', 'wss://localhost:7443', '--ca', join(hubDir, 'ca.pem'), '--code', code, '--id', 'laptop']);
  children.push(spawn(bin, ['node', 'run', '--project', proj], { env, stdio: 'ignore' }));
  await new Promise((r) => setTimeout(r, 3000));

  const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
  const page = await (await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 900 } })).newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));

  await page.goto('https://localhost:7443/');
  await page.fill('input', run(['hub', 'login', '--hub-dir', hubDir]).match(/app: (\S+) \(/)[1]);
  await page.click('button[type=submit]');
  await page.waitForSelector('text=Start an agent', { timeout: 10000 });
  check(true, 'signed in with a code');

  const start = async (label) => {
    await page.click('button:has-text("Start an agent")');
    await page.waitForSelector('.start select');
    await page.locator('.start select').nth(1).selectOption({ label });
    await page.click('.start button.button--primary');
    await page.waitForTimeout(2500);
    await page.goto('https://localhost:7443/#/');
  };

  await start('Shell');
  await page.waitForSelector('.xterm-helper-textarea', { state: 'attached', timeout: 10000 });
  await page.focus('.xterm-helper-textarea');
  await page.keyboard.type('echo hello-from-e2e\n', { delay: 5 });
  await page.waitForFunction(() => (document.querySelector('.xterm-rows')?.innerText ?? '').split('hello-from-e2e').length > 2, null, { timeout: 8000 }).catch(() => {});
  const screen = await page.innerText('.xterm-rows');
  check(screen.split('hello-from-e2e').length > 2, 'typed text reaches the shell in order and its output comes back');

  // Trust this browser on the node, then the chat agent's permission request can be answered.
  await page.goto('https://localhost:7443/#/device');
  const trust = (await page.innerText('.codebox')).trim().replace(/^agora /, '');
  run(trust.split(' '));
  await page.goto('https://localhost:7443/#/');
  await start('Fake chat agent');
  await page.waitForSelector('form textarea', { timeout: 10000 });
  await page.fill('form textarea', 'ask');
  await page.click('button:has-text("Send")');
  await page.waitForSelector('button:has-text("Allow")', { timeout: 10000 });
  check(true, 'a request sent from the window makes the chat agent ask permission, and the request shows up');
  await page.click('button:has-text("Allow")');
  await page.waitForTimeout(2500);
  check((await page.locator('button:has-text("Allow")').count()) === 0, 'allowing it (signed by this browser) clears the request');
  check((await page.locator('[role=alert]').count()) === 0, 'no error is shown');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
  await browser.close();
} catch (e) {
  console.log('FAIL', e.message);
  failed = true;
} finally {
  for (const c of children.reverse()) c.kill();
  rmSync(work, { recursive: true, force: true });
}
process.exit(failed ? 1 : 0);
