// Shared by the real-browser scripts: a throw-away hub and node on this machine, and a signed-in Chromium page.
// Needs `cargo build -p agora -p agora-fake-agent`, a built `apps/web/dist`, playwright-core (PW_DIR) and a Chromium (CHROME).
import { spawn, execFileSync } from 'node:child_process';
import { cpSync, mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

export const root = resolve(new URL('..', import.meta.url).pathname);

export async function startStack({ viewport = { width: 1440, height: 900 }, hostname, binary, web = join(root, 'apps/web/dist'), projects = [] } = {}) {
  const bin = binary ?? join(root, 'target/debug/agora');
  const fake = join(root, 'target/debug/agora-fake-agent');
  const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
  const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
  const port = Number(process.env.AGORA_TEST_PORT ?? 17443);
  const origin = `https://localhost:${port}`;
  const work = mkdtempSync(join(tmpdir(), 'agt-')); // short: Unix socket paths are limited to about 100 bytes
  const home = join(work, 'h');
  const hubDir = join(work, 'hub');
  const proj = join(work, 'proj');
  const env = { ...process.env, ...(hostname ? { HOSTNAME: hostname } : {}), AGORA_HOME: home, AGORA_LANG: 'en', GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@x', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@x' };
  const run = (args) => execFileSync(bin, args, { env }).toString();
  const children = [];
  let browser;
  const close = async () => {
    await browser?.close().catch(() => {});
    for (const c of children.reverse()) c.kill();
    rmSync(work, { recursive: true, force: true });
  };
  try {
    mkdirSync(join(home, 'manifests'), { recursive: true });
    writeFileSync(join(home, 'manifests/shell.toml'), 'kind = "shell"\ndisplay_name = "Shell"\nversion = "1"\nmin_engine = 1\n[process]\nlaunch = ["sh"]\n');
    writeFileSync(join(home, 'manifests/fake.toml'), `kind = "fake-adapter"\ndisplay_name = "Fake chat agent"\nversion = "1"\nmin_engine = 1\n[adapter]\ncommand = [${JSON.stringify(fake)}, "--adapter"]\n`);
    mkdirSync(proj);
    for (const a of [['init', '-q', '-b', 'main'], ['commit', '-q', '--allow-empty', '-m', 'init']]) execFileSync('git', ['-C', proj, ...a], { env });
    children.push(spawn(bin, ['hub', 'run', '--listen', `127.0.0.1:${port}`, '--hub-dir', hubDir, ...(web ? ['--web', web] : [])], { env, stdio: 'ignore' }));
    await new Promise((r) => setTimeout(r, 2500));
    const code = run(['hub', 'code', '--hub-dir', hubDir]).match(/[A-Z0-9]{4}-[A-Z0-9]{4}/)[0];
    run(['node', 'enroll', '--hub', `wss://localhost:${port}`, '--ca', join(hubDir, 'ca.pem'), '--code', code, '--id', 'laptop']);
    children.push(spawn(bin, ['node', 'run', '--project', proj, ...projects.flatMap(path => ['--project', path])], { env, stdio: 'ignore' }));
    await new Promise((r) => setTimeout(r, 3000));
    browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
    const page = await (await browser.newContext({ ignoreHTTPSErrors: true, viewport })).newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    page.on('console', m => { if (m.type() === 'error') console.log(m.text()); });
    await page.goto(`${origin}/`);
    await page.fill('input', run(['hub', 'login', '--hub-dir', hubDir]).match(/app: (\S+) \(/)[1]);
    await page.click('button[type=submit]');
    await page.waitForSelector('text=Start an agent', { timeout: 10000 });
    await page.goto(`${origin}/#/device`);
    await page.waitForSelector('.codebox');
    const trust = (await page.locator('.codebox').innerText()).trim().split(/\s+/);
    run(['node', 'trust', trust[3], trust[4]]);
    await page.goto(`${origin}/#/`);
    const start = async (label) => {
      await page.click('button:has-text("Start an agent")');
      await page.waitForSelector('.start select');
      await page.getByRole('button', { name: 'Use this folder', exact: true }).click();
      await page.waitForFunction(() => !!document.querySelector('.start [role=alert]') || [...document.querySelectorAll('.start button')].some(b => b.textContent === 'Start'));
      if (await page.locator('.start [role=alert]').count()) throw new Error(await page.locator('.start [role=alert]').innerText());
      await page.locator('.start select').last().selectOption({ label });
      await page.click('.start button.button--primary');
      await page.waitForTimeout(2500);
      await page.goto(`${origin}/#/`);
    };
    // A separately enrolled node, with its own home, policy, processes and cloned project folder.
    const addNode = async ({ id, project, hostname: host, projects: extraProjects = [] }) => {
      const nodeHome = join(work, `node-${id}`);
      mkdirSync(nodeHome);
      cpSync(join(home, 'manifests'), join(nodeHome, 'manifests'), { recursive: true });
      const nodeEnv = { ...env, AGORA_HOME: nodeHome, ...(host ? { HOSTNAME: host } : {}) };
      const nodeRun = args => execFileSync(bin, args, { env: nodeEnv }).toString();
      const enrollment = run(['hub', 'code', '--hub-dir', hubDir]).match(/[A-Z0-9]{4}-[A-Z0-9]{4}/)[0];
      nodeRun(['node', 'enroll', '--hub', `wss://localhost:${port}`, '--ca', join(hubDir, 'ca.pem'), '--code', enrollment, '--id', id]);
      const child = spawn(bin, ['node', 'run', '--project', project, ...extraProjects.flatMap(path => ['--project', path])], { env: nodeEnv, stdio: 'ignore' });
      children.push(child);
      await new Promise(resolve => setTimeout(resolve, 3000));
      nodeRun(['node', 'trust', trust[3], trust[4]]);
      return { run: nodeRun, env: nodeEnv, home: nodeHome, stop: () => child.kill() };
    };
    return { page, run, start, addNode, errors, close, env, bin, origin, work, proj, home, hubDir };
  } catch (e) {
    await close();
    throw e;
  }
}
