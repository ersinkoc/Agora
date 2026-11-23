// #1017 on a real node: two agents started one after the other in one folder (no separate workspace). The right panel's Changes
// lists the folder once, labelled as a shared folder, with the same files the review screen shows, also after the window reloads
// (a reload reads the hub's snapshot, where the second agent's registration of the folder once left a count of 0: "All clean").
// Uses only a throw-away project and two shells. See browser-harness.mjs for prerequisites. AGORA_SCREENSHOTS=<dir> saves both themes.
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

let stack;
let failed = false;
const check = (ok, message) => { if (!ok) throw new Error(message); console.log(`ok   ${message}`); };
const shotDir = process.env.AGORA_SCREENSHOTS;
try {
  stack = await startStack();
  const { page, origin, proj, env, errors, run } = stack;
  page.setDefaultTimeout(15_000);
  const git = (...args) => execFileSync('git', ['-C', proj, ...args], { env }).toString();

  mkdirSync(join(proj, 'src'));
  writeFileSync(join(proj, 'README.md'), '# login-fix\n\nThe sign-in service.\n');
  writeFileSync(join(proj, 'src', 'auth.rs'), 'pub fn issue() {}\n');
  git('add', '.');
  git('commit', '-q', '-m', 'files');
  writeFileSync(join(proj, 'README.md'), '# login-fix\n\nThe sign-in service, with short-lived tokens.\n');
  writeFileSync(join(proj, 'src', 'auth.rs'), 'pub fn issue(key: &[u8]) {}\n');
  writeFileSync(join(proj, 'src', 'refresh.rs'), 'pub fn refresh() {}\n');

  const panel = page.locator('#inspector-panel');
  const openChanges = async () => {
    const rail = page.locator('.rail__btn[aria-label^="Changes"]').first();
    if (await rail.getAttribute('aria-selected') !== 'true') await rail.click();
    await panel.waitFor();
  };
  const row = panel.getByRole('button', { name: /3 files changed/ });

  run(['agent', 'start', 'shell', '--cwd', proj]);
  await page.goto(`${origin}/#/`);
  await openChanges();
  // The node counts changed files every few seconds.
  await row.waitFor({ timeout: 30_000 });
  check(true, 'one agent: the panel lists the folder with its 3 changed files');

  // The second start registers the folder again; the files did not change.
  run(['agent', 'start', 'shell', '--cwd', proj]);
  await page.waitForTimeout(8_000); // more than one round of the node's Git watcher
  check(await row.count() === 1, 'two agents in one folder: the open window still lists the folder once with 3 changed files');

  await page.reload();
  await openChanges();
  await row.waitFor({ timeout: 30_000 });
  check(await row.count() === 1, 'after a reload (the hub\'s snapshot): the folder is listed once with 3 changed files');
  check((await row.innerText()).includes('Shared folder'), 'the row says the changes belong to the shared folder, not to one agent');
  check(await panel.getByText('All clean').count() === 0, 'the panel does not say All clean');
  check(await row.getAttribute('title') === `${proj} · laptop`, 'the row\'s tooltip gives the whole path and the machine');
  // The throw-away project's path is long: what leads to the folder gives way, the folder's name is shown whole inside the panel.
  const name = row.getByText('/proj', { exact: true });
  const nameBox = await name.boundingBox();
  const lead = await name.evaluate((e) => { const d = e.previousElementSibling; return d ? d.scrollWidth > d.clientWidth : false; });
  const area0 = await panel.boundingBox();
  check(lead && !!nameBox && !!area0 && nameBox.x + nameBox.width <= area0.x + area0.width && await name.evaluate((e) => e.scrollWidth <= e.clientWidth),
    'a long path is cut in the middle: the folder name stays in view');
  const count = await row.getByText('3 files changed').boundingBox();
  const area = await panel.boundingBox();
  check(!!count && !!area && count.x >= area.x && count.x + count.width <= area.x + area.width, 'the count is inside the panel, not cut off by a long path');

  if (shotDir) {
    mkdirSync(shotDir, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
      await page.waitForTimeout(400); // colours fade over 150 ms; a screenshot taken at once mixes both themes
      await page.screenshot({ path: join(shotDir, `changes-shared-folder-${theme}.png`) });
    }
  }

  // The review of the folder lists the same files the panel counts.
  await row.click();
  const list = page.getByRole('listbox', { name: 'Changed files' });
  await list.waitFor({ timeout: 30_000 });
  const files = (await list.getByRole('option').allInnerTexts()).map((s) => s.split('\n').filter(Boolean)[1] ?? s);
  check(files.length === 3 && ['README.md', 'auth.rs', 'refresh.rs'].every((n) => files.includes(n)), `the review lists the same 3 files: ${files.join(', ')}`);
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join('; ')}` : ''}`);
} catch (e) {
  failed = true;
  console.log(`FAIL ${e.message.split('\n')[0]}`);
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
