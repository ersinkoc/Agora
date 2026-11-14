// The review screen (docs/ux/review.md) on a real node: a workspace with a changed, a deleted, a renamed, a binary and a new file,
// opened from the agent's "files changed" chip. Checks the file list and its filters, one file at a time, unchanged lines shown from
// the file as it is now (and refused once the file changed), side by side, keys, and reading the changes again.
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
  const lines = (n, edit = {}) => `${Array.from({ length: n }, (_, i) => edit[i + 1] ?? `line ${i + 1}`).join('\n')}\n`;

  mkdirSync(join(proj, 'src'));
  writeFileSync(join(proj, 'src', 'app.txt'), lines(40));
  writeFileSync(join(proj, 'gone.txt'), 'bye\n');
  writeFileSync(join(proj, 'old name.txt'), 'kept as it is\n');
  git('add', '.');
  git('commit', '-q', '-m', 'files');
  writeFileSync(join(proj, 'src', 'app.txt'), lines(40, { 3: 'line three', 30: 'line thirty' }));
  git('rm', '-q', 'gone.txt');
  git('mv', 'old name.txt', 'new name.txt');
  writeFileSync(join(proj, 'notes.md'), '# notes\n\nwritten by the agent\n');
  writeFileSync(join(proj, 'blob.bin'), Buffer.from([0, 1, 2, 3, 0, 255]));

  await start('Shell');
  await page.goto(`${origin}/#/`);
  // The node counts changed files every few seconds; the chip appears with the count.
  const chip = page.getByRole('button', { name: /\d+ files changed/ }).first();
  await chip.waitFor({ timeout: 30_000 });
  await chip.click();
  const list = page.getByRole('listbox', { name: 'Changed files' });
  await list.waitFor();
  const names = async () => (await list.getByRole('option').allInnerTexts()).map((s) => s.split('\n').filter(Boolean)[1] ?? s);
  const shown = await names();
  check(['app.txt', 'blob.bin', 'gone.txt', 'new name.txt', 'notes.md'].every((n) => shown.includes(n)), `every changed file is listed: ${shown.join(', ')}`);

  const filter = (name) => page.getByRole('radiogroup', { name: 'Show' }).getByRole('radio', { name: new RegExp(`^${name}`) });
  await filter('Deleted').click();
  check(JSON.stringify(await names()) === JSON.stringify(['gone.txt']), 'the Deleted filter shows only the deleted file');
  await filter('Renamed').click();
  check(JSON.stringify(await names()) === JSON.stringify(['new name.txt']), 'the rename is one file, not a delete and an add');
  await filter('All').click();
  await page.getByRole('searchbox', { name: 'Filter by name' }).fill('app');
  check(JSON.stringify(await names()) === JSON.stringify(['app.txt']), 'the name filter narrows the list');
  await page.getByRole('searchbox', { name: 'Filter by name' }).fill('');

  await list.getByRole('option', { name: /app\.txt/ }).click();
  const body = page.locator('[data-wrap]');
  await page.getByText('line three', { exact: true }).waitFor();
  check(await page.getByText('line thirty', { exact: true }).isVisible(), 'one file shows both of its hunks');
  check(!(await body.innerText()).includes('line 15'), 'the unchanged lines between hunks are folded');
  // The file changes after the changes were read: its unchanged lines no longer belong to this diff, so they are not shown.
  writeFileSync(join(proj, 'src', 'app.txt'), lines(40, { 3: 'line three', 20: 'edited later', 30: 'line thirty' }));
  await page.getByRole('button', { name: /^Show \d+ unchanged lines$/ }).first().click();
  await page.getByText(/The file has changed since these changes were read/).waitFor();
  check(true, 'unchanged lines are refused once the file changed after the changes were read');
  await page.getByRole('button', { name: 'Read the changes again' }).click();
  await page.getByText('edited later', { exact: true }).waitFor();
  check(true, 'reading the changes again shows the new change');

  const gap = page.getByRole('button', { name: /^Show \d+ unchanged lines$/ }).first();
  const gapText = await gap.innerText();
  await gap.click();
  await page.getByText('line 15', { exact: true }).waitFor();
  check(true, `"${gapText.replace('⋯', '').trim()}" shows them from the file as it is now`);
  const row15 = body.locator('div.grid', { hasText: /^1515line 15$/ });
  check(await row15.count() === 1, 'an unfolded line is numbered on both sides');
  await page.getByRole('button', { name: 'Whole file', exact: true }).click();
  await page.getByText('line 25', { exact: true }).waitFor();
  check(!(await body.innerText()).includes('@@'), 'Whole file shows the file as one run, without hunk headings');
  await page.getByRole('button', { name: 'Whole file', exact: true }).click();

  await page.getByRole('radio', { name: 'Side by side' }).click();
  const pair = body.locator('div.grid').filter({ hasText: 'line three' });
  check((await pair.innerText()).includes('line 3') && (await pair.innerText()).includes('line three'), 'side by side puts the old and the new line in one row');
  await page.getByRole('radio', { name: 'Unified' }).click();

  await list.getByRole('option', { name: /notes\.md/ }).click();
  await page.getByText('written by the agent', { exact: true }).waitFor();
  check(true, 'a new file the agent has not added to git is shown as added');
  await list.getByRole('option', { name: /blob\.bin/ }).click();
  await page.getByText('This file is not text, so its changes are not shown.').waitFor();
  check(true, 'a binary file is named but not shown');
  await list.getByRole('option', { name: /new name\.txt/ }).click();
  await page.getByText('Only the name or the permissions of this file changed.').waitFor();
  check(await page.getByText('was old name.txt').isVisible(), 'a rename shows the old name');

  // Keys: j and k move through the list in its order (top folder first, then src/).
  await list.focus();
  await page.keyboard.press('Home');
  const before = await list.getAttribute('aria-activedescendant');
  await page.keyboard.press('j');
  const after = await list.getAttribute('aria-activedescendant');
  check(before !== after && !!after, 'j moves to the next file');
  await page.keyboard.press('k');
  check(await list.getAttribute('aria-activedescendant') === before, 'k moves back');

  await list.getByRole('option', { name: /app\.txt/ }).click();
  await page.getByText('edited later', { exact: true }).waitFor();

  if (shotDir) {
    mkdirSync(shotDir, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
      await page.waitForTimeout(400); // colours fade over 150 ms; a screenshot taken at once mixes both themes
      await page.screenshot({ path: join(shotDir, `e2e-changes-${theme}.png`) });
    }
  }
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join('; ')}` : ''}`);
} catch (e) {
  failed = true;
  console.log(`FAIL ${e.message.split('\n')[0]}`);
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
