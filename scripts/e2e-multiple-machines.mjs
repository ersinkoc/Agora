// Same portable project on separately enrolled nodes with identical host names. Only fake adapters are started.
// AGORA_TEST_BINARY + AGORA_TEST_EMBEDDED_WEB can exercise a previously installed client as a regression baseline.
import { copyFileSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { openStart, startStack } from './browser-harness.mjs';
let stack;
const check = (ok, message) => { if (!ok) throw new Error(message); console.log(`PASS: ${message}`); };
try {
  stack = await startStack({ hostname: 'shared-host', binary: process.env.AGORA_TEST_BINARY,
    ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: null } : {}) });
  const { page, run, addNode, proj, work, env, origin, errors } = stack;
  page.setDefaultTimeout(12000);
  run(['project', 'init', '--cwd', proj]);
  const copy = join(work, 'other copy');
  execFileSync('git', ['clone', '--quiet', proj, copy], { env });
  copyFileSync(join(proj, '.agent-project.json'), join(copy, '.agent-project.json'));
  writeFileSync(join(proj, 'local-only.txt'), 'local change stays on the first node');
  const desktop = await addNode({ id: 'desktop', project: copy, hostname: 'shared-host' });
  const launch = async (node, folder) => {
    await page.goto(`${origin}/#/`);
    await openStart(page);
    await page.getByRole('radio', { name: node }).click();
    await page.getByLabel('Working folder', { exact: true }).fill(folder);
    await page.getByRole('button', { name: 'Use this folder', exact: true }).click();
    await page.getByRole('button', { name: 'Start', exact: true }).waitFor();
    await page.getByRole('radio', { name: 'Fake chat agent', exact: true }).click();
    await page.getByRole('button', { name: 'Start', exact: true }).click();
    await page.locator('[role=dialog]').waitFor({ state: 'detached' });
    await page.waitForURL(/#\/agent\//);
    const id = decodeURIComponent(new URL(page.url()).hash.slice('#/agent/'.length));
    await page.locator('form textarea').fill('cwd');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    // The fake agent answers with JSON, where a Windows path's backslashes are escaped.
    await page.waitForFunction(path => document.querySelector('[aria-label="Conversation"]')?.textContent.includes(path), JSON.stringify(folder).slice(1, -1));
    return id;
  };
  const first = await launch('laptop', proj);
  const second = await launch('desktop', copy);
  await page.goto(`${origin}/#/`);
  const projectFilter = page.locator('aside select');
  await projectFilter.locator('option').nth(1).waitFor({ state: 'attached' });
  const projectOptions = await projectFilter.locator('option').allTextContents();
  check(projectOptions.length === 2 && projectOptions[1].includes('2 machine(s)'), 'one logical project counts two node identities even with identical host names');
  check(await page.locator('aside section[data-machine-id]').count() === 2, 'same-named machines remain separate in navigation');
  check(first === 'laptop/agent-1' && second === 'desktop/agent-1', 'sessions keep their own node-qualified addresses');
  check(!existsSync(join(copy, 'local-only.txt')), 'portable identity never synchronizes local edits');
  await projectFilter.selectOption(await projectFilter.locator('option').nth(1).getAttribute('value'));
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  const copyAddress = page.getByRole('button', { name: `Copy agent address ${first}`, exact: true });
  await copyAddress.click();
  check(await page.evaluate(() => navigator.clipboard.readText()) === first, 'clipboard receives the actual agent routing address');
  const shots = process.env.AGORA_SCREENSHOTS;
  const capture = async (prefix) => {
    if (!shots) return;
    mkdirSync(shots, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.screenshot({ path: join(shots, `${prefix}-${theme}.png`), fullPage: true, animations: 'disabled' });
    }
  };
  await capture('multiple-machines');
  await page.evaluate(id => { window.location.hash = `#/agent/${encodeURIComponent(id)}`; }, first);
  await page.getByRole('button', { name: `Copy agent address ${first}`, exact: true }).click();
  await page.waitForFunction(label => [...document.querySelectorAll('button')].some(button => button.getAttribute('aria-label') === label && button.textContent === 'Copied'), `Copy agent address ${first}`);
  await page.evaluate(id => { window.location.hash = `#/agent/${encodeURIComponent(id)}`; }, second);
  const nextCopy = page.getByRole('button', { name: `Copy agent address ${second}`, exact: true });
  await nextCopy.waitFor();
  check(await nextCopy.innerText() === 'Copy', 'switching focused agents clears the previous address copy status');
  await page.goto(`${origin}/#/`);
  // Clipboard refusal must still leave the selectable address and useful instructions.
  await page.evaluate(() => Object.defineProperty(navigator, 'clipboard', { configurable: true,
    value: { writeText: async () => { throw new Error('test refusal'); } } }));
  // Home shows one pane at a time, the first agent's.
  await page.getByRole('button', { name: `Copy agent address ${first}`, exact: true }).click();
  await page.getByText('Copy failed. Select the address and copy it manually.', { exact: true }).waitFor();
  check(true, 'clipboard refusal retains a selectable address and manual-copy instructions');
  await page.reload();
  await page.getByRole('button', { name: `Copy agent address ${first}`, exact: true }).waitFor();
  check(await page.locator('aside section[data-machine-id]').count() === 2, 'snapshot reload preserves machine identity and session addresses');
  await page.goto(`${origin}/#/machines`);
  // Each machine's card names its enrolled ID under the shared host name.
  await page.getByText('laptop', { exact: true }).waitFor();
  await page.getByText('desktop', { exact: true }).waitFor();
  check(true, 'machine details expose enrollment identities separately from host names');
  await page.addInitScript(() => Object.defineProperty(navigator, 'language', { get: () => 'tr-TR' }));
  await page.goto(`${origin}/#/`);
  await page.reload();
  await page.getByRole('button', { name: `Ajan adresini kopyala ${first}`, exact: true }).waitFor();
  check(await page.locator('main section header strong').evaluateAll(names => names.every(name => name.getBoundingClientRect().width > 50)),
    'localized pane headers keep agent names readable beside the actions');
  await capture('multiple-machines-turkish');
  check(true, 'Turkish agent addresses and copy actions render in both themes');
  desktop.stop();
  await page.locator('aside section[data-machine-id="desktop"]').getByText('Çevrimdışı', { exact: true }).waitFor({ state: 'attached' });
  check(await page.locator('aside section[data-machine-id="laptop"]').getByText('Çevrimiçi', { exact: true }).count() === 1,
    'disconnecting one same-named node never marks the other node offline');
  check(errors.length === 0, 'no browser errors');
} catch (e) {
  console.error(e.stack || e); if (stack) console.error((await stack.page.locator('body').innerText()).slice(0, 4000));
  process.exitCode = 1;
} finally { await stack?.close(); }
