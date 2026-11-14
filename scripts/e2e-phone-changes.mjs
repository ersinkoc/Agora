// The phone window (#484, docs/ux/phone.md) on a real node: a running agent's screen offers what it changed and its files, one tap
// each, and they open the review screen and the file browser full-screen. Signs in at desktop size, then shrinks to a phone.
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
  const { page, origin, proj, env, errors, run } = stack;
  page.setDefaultTimeout(20_000);
  const git = (...args) => execFileSync('git', ['-C', proj, ...args], { env }).toString();
  writeFileSync(join(proj, 'app.txt'), 'one\ntwo\nthree\n');
  git('add', '.');
  git('commit', '-q', '-m', 'files');
  writeFileSync(join(proj, 'app.txt'), 'one\nTWO\nthree\n');
  writeFileSync(join(proj, 'notes.md'), 'new\n');
  run(['agent', 'start', 'shell', '--cwd', proj]);

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${origin}/#/`);
  await page.getByText(/shell · /).first().click();
  await page.waitForURL(/#\/agent\//);
  // The node counts changed files every few seconds; the button appears with the count.
  const changes = page.getByRole('button', { name: /^2 files changed$/ });
  await changes.waitFor({ timeout: 40_000 });
  check(true, 'a running agent\'s phone screen offers its 2 changed files');
  await changes.click();
  const list = page.getByRole('listbox', { name: 'Changed files' });
  await list.waitFor();
  check((await list.getByRole('option').count()) === 2, 'the review screen opens full-screen with both files');
  await page.getByText('TWO', { exact: true }).waitFor();
  check(true, 'and shows the change');
  if (shotDir) { mkdirSync(shotDir, { recursive: true }); await page.screenshot({ path: join(shotDir, 'e2e-phone-review.png') }); }
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByRole('button', { name: /^Files$/ }).click();
  const tree = page.getByRole('tree', { name: 'Folders and files' });
  await tree.getByRole('treeitem', { name: /^notes\.md/ }).waitFor();
  check(true, 'the Files button opens the file browser');
  await tree.getByRole('treeitem', { name: /^app\.txt/ }).click();
  await page.getByRole('region', { name: 'app.txt' }).getByText('TWO', { exact: true }).waitFor();
  check(true, 'a file opens with its text');
  if (shotDir) await page.screenshot({ path: join(shotDir, 'e2e-phone-files.png') });
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  check(/#\/agent\//.test(page.url()), 'closing it returns to the agent');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join('; ')}` : ''}`);
} catch (e) {
  failed = true;
  console.log(`FAIL ${e.message.split('\n')[0]}`);
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
