// ADR-0052: selected/non-Git folders, explicit identity, shared sessions and opt-in isolation in a real browser.
// Uses only throw-away folders, a shell and agora-fake-agent. See browser-harness.mjs for prerequisites.
import { mkdirSync, readFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

let stack;
const check = (ok, message) => { if (!ok) throw new Error(message); console.log(`ok   ${message}`); };
const shotDir = process.env.AGORA_SCREENSHOTS;
try {
  stack = await startStack();
  const { page, origin, work, proj, env, errors } = stack;
  page.setDefaultTimeout(10_000);
  const folder = join(work, 'shared folder');
  mkdirSync(folder); // Appears after the node has started, and is outside Git.
  const selectFolder = async (path, create = false) => {
    await page.goto(`${origin}/#/`);
    await page.getByRole('button', { name: 'Start an agent', exact: true }).click();
    await page.getByLabel('Working folder', { exact: true }).fill(path);
    if (create) await page.getByLabel('Create a shared project identity', { exact: true }).check();
    await page.getByRole('button', { name: 'Use this folder', exact: true }).click();
    await page.getByRole('button', { name: 'Start', exact: true }).waitFor();
  };
  const launchSelected = async () => {
    await page.getByRole('button', { name: 'Start', exact: true }).click();
    await page.locator('[role=dialog]').waitFor({ state: 'detached' });
    await page.waitForURL(/#\/agent\//);
  };
  const screenshots = async name => {
    if (!shotDir) return;
    mkdirSync(shotDir, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.screenshot({ path: join(shotDir, `${name}-${theme}.png`), fullPage: true });
    }
  };
  await selectFolder(folder, true);
  check(existsSync(join(folder, '.agent-project.json')), 'explicit selection creates portable project metadata');
  check(!!JSON.parse(readFileSync(join(folder, '.agent-project.json'))).project_id, 'metadata contains a project_id');
  check(await page.getByLabel('Create an isolated working copy', { exact: true }).count() === 0, 'non-Git folder offers no worktree option');
  await screenshots('selected-folder');
  await page.getByRole('radio', { name: 'Fake chat agent', exact: true }).click();
  await launchSelected();
  await page.waitForSelector('form textarea');
  await page.fill('form textarea', 'cwd');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await page.waitForFunction(path => document.querySelector('[aria-label="Conversation"]')?.textContent.includes(path), folder);
  check(true, 'adapter process and session report the selected folder as cwd');

  await selectFolder(folder);
  await page.getByRole('radio', { name: 'Fake chat agent', exact: true }).click();
  await launchSelected();
  await page.waitForSelector('form textarea');
  await page.goto(`${origin}/#/`);
  await page.getByText('Shared folder · changes belong to the workspace', { exact: true }).first().waitFor();
  check(true, 'two independent sessions in one folder are marked as sharing files');
  await page.reload();
  await page.getByText('Shared folder · changes belong to the workspace', { exact: true }).first().waitFor();
  check(true, 'workspace/session binding survives a browser reload');
  await screenshots('shared-sessions');

  const git = args => execFileSync('git', ['-C', proj, ...args], { env }).toString();
  const before = git(['worktree', 'list', '--porcelain']);
  await selectFolder(proj);
  const isolated = page.getByLabel('Create an isolated working copy', { exact: true });
  check(!await isolated.isChecked(), 'Git launch defaults to the existing working folder');
  await screenshots('git-folder');
  await isolated.check();
  await page.getByRole('radio', { name: 'Shell', exact: true }).click();
  await launchSelected();
  await page.waitForSelector('.xterm-helper-textarea', { state: 'attached' });
  check(git(['worktree', 'list', '--porcelain']) !== before, 'explicit isolated copy creates a worktree');
  check(git(['branch', '--show-current']).trim() === 'main', 'original branch is preserved');
  check(!existsSync(join(proj, '.agent-project.json')), 'default launch never creates portable metadata');
  await page.addInitScript(() => Object.defineProperty(navigator, 'language', { get: () => 'tr-TR' }));
  await page.goto(`${origin}/#/`);
  await page.reload();
  await page.getByRole('button', { name: 'Ajan başlat', exact: true }).click();
  await page.getByLabel('Çalışma klasörü', { exact: true }).fill(proj);
  await page.getByRole('button', { name: 'Bu klasörü kullan', exact: true }).click();
  await page.getByRole('button', { name: 'Başlat', exact: true }).waitFor();
  await screenshots('turkish-folder');
  check(true, 'Turkish launch flow renders with localized labels');
  check(errors.length === 0, `no browser errors${errors.length ? ': ' + errors.join(', ') : ''}`);
} catch (e) {
  console.error('FAIL', e.stack || e.message);
  if (stack) console.error((await stack.page.locator('body').innerText()).slice(0, 6000));
  process.exitCode = 1;
} finally {
  await stack?.close();
}
