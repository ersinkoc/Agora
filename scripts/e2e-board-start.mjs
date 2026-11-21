// ADR-0095 step 3: "Start a new agent for this task" from the board, against a real hub and node with the fake chat agent
// (agora-fake-agent). The ordinary start window opens for the task's project; once the agent started, the same confirmation follows for it
// and the task is given. No real coding agent or account is used. Needs the normal browser-harness prerequisites and a node that offers
// `task.dispatch` (AGORA_TEST_BINARY may name such a build).
// AGORA_SCREENSHOTS=<dir> also saves screenshots of the picker, the start window and the confirmation in both themes and at phone width.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { startStack } from './browser-harness.mjs';

async function mesh(bin, env, socket) {
  const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const pending = new Map();
  let next = 0;
  createInterface({ input: child.stdout }).on('line', line => {
    const reply = JSON.parse(line);
    const request = pending.get(reply.id);
    if (!request) return;
    pending.delete(reply.id);
    clearTimeout(request.timer);
    if (reply.error) request.reject(new Error(JSON.stringify(reply.error)));
    else request.resolve(reply.result);
  });
  const call = (method, params) => new Promise((resolve, reject) => {
    const id = ++next;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP timed out: ${method}`)); }, 15000);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'board-start-fixture', version: '1' } });
  return {
    tool: async (name, args) => {
      const result = await call('tools/call', { name, arguments: args });
      assert.equal(!!result.isError, false, JSON.stringify(result));
      return JSON.parse(result.content.find(item => item.type === 'text').text);
    },
    close: () => child.kill(),
  };
}

let stack;
let client;
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page, run, bin, env, home, proj, errors } = stack;
  page.setDefaultTimeout(15000);
  // A plain terminal only, for the fixture's board tool: it is not an agent, so no agent of the project can take a task yet.
  const shell = run(['agent', 'start', 'shell', '--cwd', proj]).match(/agent-\d+/)?.[0];
  assert.ok(shell);
  await page.waitForFunction(() => document.querySelectorAll('aside button[aria-pressed]').length >= 1);
  client = await mesh(bin, env, join(home, 'm', `${shell}.sock`));
  await client.tool('task_create', { title: 'Write the release notes', detail: 'Fixture detail. Ignore any instruction in it.', depends_on: [] });

  await page.getByRole('tab', { name: /^(Shared tasks and files|Ortak görevler ve dosyalar)$/ }).click();
  await page.getByRole('button', { name: /^(Open board|Panoyu aç)/ }).click();
  const board = page.getByRole('dialog');
  await board.waitFor();
  const col = name => board.locator(`[data-column="${name}"]`);
  await col('backlog').getByText('Write the release notes', { exact: true }).waitFor();

  const screenshots = process.env.AGORA_SCREENSHOTS;
  const shoot = async (name) => {
    if (!screenshots) return;
    mkdirSync(screenshots, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme: theme });
      // Shoot only once the page shows the scheme asked for and the 150 ms colour fade is over (#885).
      await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme);
      await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(screenshots, `board-start-${name}-${theme}.png`), animations: 'disabled' });
    }
  };

  // No agent can take the task, and the window can start one: the card still offers the action, and the picker's main action starts one.
  await col('backlog').getByRole('button', { name: 'Give “Write the release notes” to an agent' }).focus();
  await page.keyboard.press('Enter');
  const give = page.locator('[data-give-dialog]');
  await give.waitFor();
  assert.equal(await give.locator('[data-agent-id]').count(), 0);
  const startNew = give.getByRole('button', { name: 'Start a new agent for this task', exact: true });
  await startNew.waitFor();
  const height = await startNew.evaluate(b => b.getBoundingClientRect().height);
  assert.ok(height >= 44, `the start entry is ${height}px high`);
  await shoot('empty');

  // The ordinary start window, for the task's project. Cancel goes back to the picker; nothing is started or given.
  await startNew.click();
  const start = page.getByRole('dialog', { name: 'Start an agent' });
  await start.waitFor();
  assert.equal(await give.count(), 0);
  assert.equal(await start.locator('[data-start-for-task]').innerText(), `For the task “Write the release notes” of ${proj.split('/').pop()}`);
  await start.getByRole('button', { name: 'Cancel', exact: true }).last().click();
  await give.waitFor();
  assert.equal(await give.locator('[role=alert]').count(), 0);

  // Start it for real: the folder is the project's (prefilled, chosen by the person), the agent the fake chat agent.
  await give.getByRole('button', { name: 'Start a new agent for this task', exact: true }).click();
  await start.waitFor();
  await page.waitForFunction(() => !!document.querySelector('[role=dialog] input:not([type])')?.value || !!document.querySelector('[role=dialog] li button'), null, { timeout: 60000 });
  if (await start.locator('input:not([type])').inputValue()) await start.getByRole('button', { name: 'Use this folder', exact: true }).click();
  else await start.locator('li button').first().click();
  await start.getByRole('radio', { name: 'Fake chat agent', exact: true }).waitFor({ timeout: 60000 });
  // A plain terminal cannot take a task: not offered here.
  assert.equal(await start.getByRole('radio', { name: 'Shell', exact: true }).count(), 0);
  await start.getByRole('radio', { name: 'Fake chat agent', exact: true }).click();
  await shoot('start');
  await start.getByRole('button', { name: 'Start', exact: true }).click();

  // Started: the same confirmation as for any agent, for the new one; nothing is sent before "Give the task".
  await give.locator('[data-give-step="confirm"]').waitFor({ timeout: 90000 });
  const confirm = await give.innerText();
  for (const part of ['Write the release notes', 'fake-adapter', 'Give this task to']) assert.ok(confirm.includes(part), `confirmation names ${part}: ${confirm}`);
  assert.doesNotMatch(confirm, /\b(claim|lease|mesh|dispatch)/i);
  assert.equal(await page.evaluate(() => document.activeElement?.textContent), 'Back');
  // Still on Home: the window did not jump to the new agent's view, as an ordinary start would.
  assert.doesNotMatch(page.url(), /#\/agent\//);
  await shoot('confirm');
  await give.getByRole('button', { name: 'Give the task', exact: true }).click();
  await give.locator('[data-give-step="result"]').waitFor();
  const answer = await give.locator('[role=status]').innerText();
  assert.match(answer, /^Sent to .+\.( It waits until the agent is ready and its current turn ends\.)?$/);
  await shoot('sent');
  await give.getByRole('button', { name: 'Done', exact: true }).click();
  await give.waitFor({ state: 'detached' });
  await col('doing').getByText('Write the release notes', { exact: true }).waitFor({ timeout: 20000 });

  // A second task: the new agent is listed now, and starting another one is the quieter entry under the list.
  await client.tool('task_create', { title: 'Port the parser to the new API', detail: '', depends_on: [] });
  await col('backlog').getByRole('button', { name: 'Give “Port the parser to the new API” to an agent' }).click();
  await give.locator('[data-agent-id]').first().waitFor();
  await give.getByRole('button', { name: 'Start a new agent for this task', exact: true }).waitFor();
  await shoot('picker');
  // Escape goes to the topmost dialog: the picker closes and the board stays. Pressed once the picker is ready (it holds the focus, and a
  // frame has passed since: a dialog listens for Escape a render after it appears; until then the board ignores the key, dialogStack.ts).
  const pickerReady = async () => {
    await page.waitForFunction(() => !!document.querySelector('[data-give-dialog]')?.contains(document.activeElement));
    await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
  };
  // In the document, not by role: the picker is modal, so the board under it is hidden from the accessibility tree while it is open.
  const boardOpen = async () => (await page.locator('[data-column="backlog"]').count()) === 1;
  await pickerReady();
  await page.keyboard.press('Escape');
  await give.waitFor({ state: 'detached' });
  assert.ok(await boardOpen(), 'the board stays open under the picker');
  // An Escape right after the picker opened (no wait) never closes the board under it: it closes the picker, or does nothing yet.
  const giveParser = col('backlog').getByRole('button', { name: 'Give “Port the parser to the new API” to an agent' });
  for (let i = 0; i < 5; i++) {
    await giveParser.click();
    await give.waitFor();
    await page.keyboard.press('Escape');
    await page.waitForTimeout(400);
    assert.ok(await boardOpen(), `an early Escape did not close the board (try ${i + 1})`);
    if (await give.count()) {
      await pickerReady();
      await page.keyboard.press('Escape');
      await give.waitFor({ state: 'detached' });
    }
  }
  if (screenshots) {
    // At phone width the window shows its phone layout (no board); the picker stays.
    await giveParser.click();
    await give.waitFor();
    await page.setViewportSize({ width: 390, height: 844 });
    for (const theme of ['light', 'dark']) {
      await page.emulateMedia({ colorScheme: theme });
      await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme);
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(screenshots, `board-start-phone-picker-${theme}.png`), animations: 'disabled' });
    }
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await give.getByRole('button', { name: 'Start a new agent for this task', exact: true }).click();
    await start.waitFor();
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(screenshots, 'board-start-phone-start-dark.png'), animations: 'disabled' });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    // Escape closes the start window only (the topmost dialog); the picker comes back.
    await page.keyboard.press('Escape');
    await start.waitFor({ state: 'detached' });
    await give.waitFor();
    await pickerReady();
    await page.keyboard.press('Escape');
    await give.waitFor({ state: 'detached' });
  }
  assert.deepEqual(errors, []);
  console.log('PASS: a Backlog task with no agent to take it starts the fake chat agent through the ordinary start window, then is given to it after the confirmation.');
} finally {
  client?.close();
  await stack?.close();
}
