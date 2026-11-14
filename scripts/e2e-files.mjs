// The file browser (docs/ux/main-window.md, Files) on a real node, in its window and in a pane: the folder tree with the changed
// files and folders marked, open files as tabs, the path bar, change marks in the gutter, search in a file, the changed-only list,
// a 20 000-line file drawn a screenful at a time, and letters typed in the view staying there (the window's `n` must not act).
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

  mkdirSync(join(proj, 'src', 'deep'), { recursive: true });
  writeFileSync(join(proj, 'src', 'deep', 'core.txt'), lines(30));
  writeFileSync(join(proj, 'src', 'keep.txt'), 'unchanged\n');
  writeFileSync(join(proj, 'big.txt'), lines(20_000));
  writeFileSync(join(proj, 'README.md'), '# project\n');
  writeFileSync(join(proj, 'src', 'app.ts'), 'export const greeting = "hello"; // said once\n');
  git('add', '.');
  git('commit', '-q', '-m', 'files');
  writeFileSync(join(proj, 'src', 'deep', 'core.txt'), lines(30, { 5: 'line five', 20: 'needle here' }));
  writeFileSync(join(proj, 'notes.md'), 'new notes\n');

  await start('Shell');
  const agentKey = decodeURIComponent(page.url().split('#/agent/')[1]);
  await page.goto(`${origin}/#/`);
  await page.getByRole('button', { name: 'Files', exact: true }).first().click();
  const tree = page.getByRole('tree', { name: 'Folders and files' });
  await tree.waitFor();
  const item = (name) => tree.getByRole('treeitem', { name: new RegExp(`^${name.replace('.', '\\.')}`) });
  await item('src').waitFor();
  // The marks arrive with the workspace's diff, read alongside the folders.
  check(await item('src').locator('[title="Holds changed files"]').waitFor().then(() => true, () => false), 'a folder holding a changed file is marked');
  check(await item('notes.md').getByText('A', { exact: true }).waitFor().then(() => true, () => false), 'a new file carries its status letter');
  await item('src').click();
  await item('deep').click();
  await item('core.txt').click();
  const code = page.getByRole('region', { name: 'src/deep/core.txt' });
  await code.getByText('needle here', { exact: true }).waitFor();
  check(await code.locator('[title="Changed since the last commit"]').count() === 2, 'the two changed lines are marked in the gutter');
  const crumbs = page.getByRole('navigation', { name: 'Path' });
  check((await crumbs.innerText()).replace(/\s+/g, '') === 'srcdeepcore.txt', 'the path bar shows the folders and the file');

  // Letters typed in the view are its own: `n` (the window's "next that needs you") must not take the window elsewhere.
  const url = page.url();
  await code.focus();
  await page.keyboard.press('n');
  await page.waitForTimeout(300);
  check(page.url() === url && await tree.isVisible(), 'a letter typed in the file view does not trigger the window shortcut');

  await page.keyboard.press('Control+f');
  const find = page.getByRole('searchbox', { name: 'Find in the file' });
  await find.waitFor();
  await find.fill('needle');
  await page.getByText('1 of 1').waitFor();
  check(await code.locator('mark').count() === 1, 'search finds and marks the match');
  await page.keyboard.press('Escape');

  // Syntax colours: a TypeScript file gets the highlighter's classes (loaded on demand, after the plain text is shown).
  await item('app.ts').click();
  const ts = page.getByRole('region', { name: 'src/app.ts' });
  check(await ts.locator('.hljs-string').first().waitFor().then(() => true, () => false) && await ts.locator('.hljs-comment').count() === 1, 'a TypeScript file is shown with syntax colours');
  check((await ts.innerText()).includes('export const greeting = "hello"; // said once'), 'highlighting leaves the text as it is');
  await page.getByRole('button', { name: 'Close app.ts' }).click();
  await item('keep.txt').click();
  const tabs = page.getByRole('tablist', { name: 'Open files' });
  check(await tabs.getByRole('tab').count() === 2, 'each opened file has a tab');
  await page.getByRole('button', { name: 'Close keep.txt' }).click();
  check(await tabs.getByRole('tab').count() === 1 && await code.isVisible(), 'closing a tab shows the one before it');

  await item('big.txt').click();
  const big = page.getByRole('region', { name: 'big.txt' });
  await big.getByText('line 1', { exact: true }).waitFor();
  const drawn = await big.locator('div.flex').count();
  check(drawn < 200, `a 20 000-line file draws only what is near the view (${drawn} rows)`);
  await big.evaluate((el) => { el.scrollTop = el.scrollHeight; });
  await big.getByText('line 20000', { exact: true }).waitFor();
  check(true, 'scrolling to the end shows its last line');

  await page.getByRole('button', { name: '2 changed', exact: true }).click();
  const flat = await tree.getByRole('treeitem').allInnerTexts();
  check(flat.length === 2 && flat.some((s) => s.includes('src/deep/core.txt')) && flat.some((s) => s.includes('notes.md')), 'the changed-only list names exactly the changed files');
  await page.getByRole('button', { name: '2 changed', exact: true }).click();
  await page.keyboard.press('Escape');
  await page.getByRole('dialog').waitFor({ state: 'detached' });

  // In a pane: the agent's files next to it, from the pane's menu.
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Two side by side', exact: true }).click();
  await page.waitForFunction(() => document.querySelectorAll('[data-pane]').length === 2);
  await page.getByRole('button', { name: /^Pane options/ }).last().click();
  await page.getByRole('menuitem', { name: 'Show…' }).click();
  const shell = page.getByRole('menuitemradio').filter({ hasText: /shell/ }).first();
  await shell.click();
  await page.getByRole('button', { name: /^Pane options/ }).last().click();
  await page.getByRole('menuitem', { name: 'Show…' }).click();
  await page.getByRole('menuitemradio', { name: "This agent's files" }).click();
  const paneTree = page.getByRole('tree', { name: 'Folders and files' });
  await paneTree.waitFor();
  check(await paneTree.getByRole('treeitem', { name: /^src/ }).waitFor().then(() => true, () => false), `a pane shows ${agentKey}'s files beside its terminal`);

  if (shotDir) {
    mkdirSync(shotDir, { recursive: true });
    await paneTree.getByRole('treeitem', { name: /^notes\.md/ }).click();
    for (const theme of ['dark', 'light']) {
      await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
      await page.waitForTimeout(400); // colours fade over 150 ms; a screenshot taken at once mixes both themes
      await page.screenshot({ path: join(shotDir, `e2e-files-pane-${theme}.png`) });
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
