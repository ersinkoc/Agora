// The tool panes (docs/ux/main-window.md, "Tool panes") on a real node: Changes, File manager, Files and Terminal as panes of the
// window, opened from the command palette, the file manager's list, sort, preview and switched-off write actions, a terminal started
// from "Start a new terminal" that opens as a pane, and the panes coming back after a reload.
// Uses only a throw-away project and a shell. See browser-harness.mjs for prerequisites. AGORA_SCREENSHOTS=<dir> saves both themes.
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
  const { page, origin, proj, env, errors, start } = stack;
  page.setDefaultTimeout(15_000);
  const git = (...args) => execFileSync('git', ['-C', proj, ...args], { env }).toString();
  mkdirSync(join(proj, 'src'));
  writeFileSync(join(proj, 'src', 'app.ts'), 'export const a = 1;\n');
  writeFileSync(join(proj, 'README.md'), '# project\n');
  git('add', '.');
  git('commit', '-q', '-m', 'files');
  writeFileSync(join(proj, 'src', 'app.ts'), 'export const a = 2;\n');
  writeFileSync(join(proj, 'notes.md'), 'new notes\n');

  await start('Shell');
  await page.goto(`${origin}/#/`);
  await page.getByRole('tab', { name: /shell/ }).first().waitFor();
  const palette = async (text) => { await page.keyboard.press('Control+k'); await page.keyboard.type(text); await page.keyboard.press('Enter'); };

  // Changes: with one folder on the machine the pane opens on it (no question to answer).
  await palette('Open Changes beside');
  const list = page.getByRole('listbox', { name: 'Changed files' });
  await list.waitFor();
  check(await list.getByRole('option').count() === 2, 'the Changes pane lists the two changed files');
  await page.getByRole('option', { name: /app\.ts/ }).first().click();
  check(await page.getByText('export const a = 2;').first().waitFor().then(() => true, () => false), 'the diff of the chosen file is beside the list');

  // File manager: list, sort, filter, folder, preview, and what it will not do.
  await palette('Open the File manager beside');
  const grid = page.getByRole('grid', { name: 'Files in this folder' });
  await grid.waitFor();
  await grid.getByRole('row', { name: /^src/ }).waitFor();
  check(await grid.getByRole('row').count() >= 4, 'the File manager lists the folder (folders first)');
  const firstName = () => grid.locator('[role=row][data-path]').first().getAttribute('data-path');
  check(await firstName() === 'src', 'a folder comes first');
  await grid.getByRole('columnheader', { name: 'Size' }).getByRole('button').click();
  check(await grid.getByRole('columnheader', { name: 'Size' }).getAttribute('aria-sort') === 'ascending', 'a click on Size sorts by size');
  check(await firstName() === 'src', 'folders stay first when sorted');
  await page.locator('#fm-filter').fill('readme');
  check(await grid.locator('[role=row][data-path]').count() === 1, 'the name filter narrows the list');
  await page.locator('#fm-filter').fill('');
  await grid.locator('[role=row][data-path="src"]').click();
  await grid.locator('[role=row][data-path="src/app.ts"]').click();
  const preview = page.getByRole('region', { name: 'Preview' }).or(page.locator('section[aria-label=Preview]'));
  await preview.getByText('const a = 2').first().waitFor();
  check(true, 'a file previews with its text');
  for (const name of ['Rename', 'Move', 'Delete', 'Upload']) {
    check(await page.getByRole('button', { name }).first().getAttribute('aria-disabled') === 'true', `${name} is shown switched off`);
  }
  await page.keyboard.press('Escape');
  check(await preview.count() === 0, 'Escape closes the preview');

  // The Files pane and the agent's terminal as a pane.
  await palette('Open Files beside');
  await page.getByRole('tree', { name: 'Folders and files' }).waitFor();
  check(true, 'the Files pane shows the folder as a tree');
  await palette('Open the terminal beside');
  await page.waitForTimeout(500);

  // A terminal started from the palette is a pane of the window, not a new page.
  await palette('Start a new terminal');
  const dialog = page.getByRole('dialog');
  await dialog.waitFor();
  await page.waitForFunction(() => !!document.querySelector('[role=dialog] input:not([type])')?.value || !!document.querySelector('[role=dialog] li button'), null, { timeout: 60000 });
  if (await dialog.locator('input:not([type])').inputValue()) await dialog.getByRole('button', { name: 'Use this folder', exact: true }).click();
  else await dialog.locator('li button').first().click();
  await page.waitForFunction(() => [...document.querySelectorAll('[role=dialog] button')].some((b) => b.textContent === 'Start'), null, { timeout: 60000 });
  check(await dialog.getByRole('radio', { name: 'Shell', exact: true }).getAttribute('aria-checked') === 'true', 'the start window opens on the Shell kind');
  await dialog.getByRole('button', { name: 'Start', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('[role=dialog]'), null, { timeout: 90000 });
  await page.locator('[data-pane] .xterm').first().waitFor({ timeout: 30000 });
  check(page.url().endsWith('#/'), 'the window stayed (no full-page terminal)');
  check(await page.locator('[data-pane]').filter({ hasText: /Terminal/ }).count() >= 1, 'the new terminal is a pane');
  if (shotDir) { mkdirSync(shotDir, { recursive: true }); await page.screenshot({ path: join(shotDir, 'panes-real.png') }); }

  // Remembered on this device.
  await page.waitForTimeout(500); // the layout is written a moment after the last change
  await page.reload();
  await page.getByRole('grid', { name: 'Files in this folder' }).waitFor();
  await page.getByRole('listbox', { name: 'Changed files' }).waitFor();
  check(await page.getByRole('listbox', { name: 'Changed files' }).count() === 1 && await page.locator('[data-pane]').count() >= 4, 'the panes (Changes, File manager, Files, the new terminal) come back after a reload');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join('; ')}` : ''}`);
} catch (e) {
  failed = true;
  console.log(`FAIL ${e.message.split('\n')[0]}`);
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
