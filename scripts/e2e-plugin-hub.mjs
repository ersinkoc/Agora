// End-to-end check of a plugin installed on the hub (ADR-0114 decision 6, build step 3) with a real hub, node and window: the owner
// installs the sample plugin on the hub's machine (`agora hub plugin add`); the window lists it as the hub's (not "development"), offers
// none of its panes until the person shows it, keeps the person's grant on the hub (not in the browser) so a reload draws it at once with
// the real agents, and drops it at the next load once the owner removed it (`agora hub plugin remove`).
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run gen && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-plugin-hub.mjs
//
// AGORA_BIN names another agora binary (default target/debug/agora); AGORA_TEST_PORT another port for the hub (default 17443).
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';
import { pack } from './pack-plugin.mjs';

const root = new URL('..', import.meta.url).pathname;
let failed = 0;
let total = 0;
const check = (ok, what) => { total++; console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const layout = JSON.stringify({ version: 1, layout: { version: 1, root: { type: 'pane', id: 'p1', panel: 'plugin.sample-agents.list' } }, focus: 'p1', touched: true });
let stack;
try {
  stack = await startStack({ ...(process.env.AGORA_BIN ? { binary: process.env.AGORA_BIN } : {}) });
  const { page, run, hubDir, work, proj, errors } = stack;
  page.setDefaultTimeout(15000);
  run(['agent', 'start', 'shell', '--cwd', proj]);
  const file = join(work, 'sample-agents.json');
  writeFileSync(file, pack(join(root, 'apps/web/plugins/sample')));
  const added = run(['hub', 'plugin', 'add', file, '--hub-dir', hubDir]);
  check(/Installed Agent list 0\.1\.0 \(sample-agents\)/.test(added), `the owner installs it on the hub's machine (${added.trim()})`);

  const plugins = async () => {
    await page.keyboard.press('Control+KeyK');
    await page.keyboard.type('Plugins');
    await page.keyboard.press('Enter');
    await page.waitForSelector('[data-plugins-dialog]');
  };
  const done = () => page.getByRole('button', { name: 'Done' }).click();
  const frames = () => page.locator('iframe[data-plugin-frame="sample-agents"]').count();

  // The window was open before the plugin was installed: it is offered at the next load.
  await page.evaluate((l) => localStorage.setItem('agora.workspace.v1', l), layout);
  await page.reload();
  await page.waitForSelector('[data-layout-root]');
  await plugins();
  const row = page.locator('[data-plugin-row="sample-agents"]');
  await row.waitFor().catch(async (e) => { throw new Error(`${e.message.split('\n')[0]}; the dialog: ${(await page.locator('[data-plugins-dialog]').innerText()).replace(/\s+/g, ' ')}`); });
  const text = await row.innerText();
  check(text.includes('installed on this hub') && !text.includes('development') && !(await row.getByRole('button', { name: 'Remove' }).count()),
    `the window lists it as the hub's, without Remove (${text.replace(/\s+/g, ' ')})`);
  await done();
  check((await frames()) === 0, 'none of its panes is drawn before the person shows it');

  await plugins();
  await row.getByRole('button', { name: 'Look at it' }).click();
  await page.locator('input[data-cap="agents.read"]').check();
  await page.locator('[data-plugin-save]').click();
  await row.getByText('It sees one thing you allowed').waitFor();
  await done();
  const frame = page.frameLocator('iframe[data-plugin-frame="sample-agents"]');
  await frame.locator('li').first().waitFor({ timeout: 15000 }).catch(() => {});
  const rows = await frame.locator('li').count();
  const dev = await page.locator('iframe[data-plugin-frame="sample-agents"]').getAttribute('data-plugin-dev');
  check(rows > 0 && dev === null, `once shown, its pane draws the real agents (${rows}), not marked development`);

  // The grant is the hub's, not this browser's: a reload draws it at once, and nothing about it is in this browser's storage.
  await page.reload();
  await page.waitForSelector('[data-layout-root]');
  await page.frameLocator('iframe[data-plugin-frame="sample-agents"]').locator('li').first().waitFor({ timeout: 15000 }).catch(() => {});
  const local = await page.evaluate(() => localStorage.getItem('agora.plugins.grants.v1'));
  check((await frames()) === 1 && !(local ?? '').includes('sample-agents'), `after a reload it is drawn with the grant the hub kept (browser grants: ${local})`);

  const removed = run(['hub', 'plugin', 'remove', 'sample-agents', '--hub-dir', hubDir]);
  check(removed.includes('Removed sample-agents'), `the owner removes it (${removed.trim()})`);
  await page.reload();
  await page.waitForSelector('[data-layout-root]');
  await page.waitForTimeout(2000);
  const placeholder = await page.locator('[data-layout-root]').innerText();
  await plugins();
  check((await frames()) === 0 && (await row.count()) === 0 && placeholder.includes('plugin.sample-agents.list'),
    'at the next load it is gone from the window, its pane a placeholder');
  check(errors.length === 0, `no page errors (${errors.join('; ')})`);
} catch (e) {
  check(false, `the run went through: ${e.message.split('\n')[0]}`);
} finally {
  await stack?.close();
}
console.log(`\n${total - failed}/${total} passed`);
process.exit(failed ? 1 : 0);
