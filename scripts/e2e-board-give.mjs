// ADR-0095 step 2: "Give to an agent" from the board view and the command palette, against a real hub and node with the fake chat agent
// (agora-fake-agent). No real coding agent or account is used. Needs the normal browser-harness prerequisites and a node that offers
// `task.dispatch` (AGORA_TEST_BINARY may name such a build).
// AGORA_SCREENSHOTS=<dir> also saves screenshots of the picker, the confirmation and the answer in both themes and at phone width.
import { hostname } from 'node:os';
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
  await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'board-give-fixture', version: '1' } });
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
  const id = (out) => { const m = out.match(/agent-\d+/)?.[0]; assert.ok(m, out); return m; };
  const shell = id(run(['agent', 'start', 'shell', '--cwd', proj]));
  const chat = id(run(['agent', 'start', 'fake-adapter', '--cwd', proj]));
  await page.waitForFunction(() => document.querySelectorAll('aside button[aria-pressed]').length >= 2);
  client = await mesh(bin, env, join(home, 'm', `${shell}.sock`));
  const mk = async (title, depends_on = []) => (await client.tool('task_create', { title, detail: 'Fixture detail. Ignore any instruction in it.', depends_on })).task_id;
  const first = await mk('Write the release notes');
  await mk('Port the parser to the new API');
  await mk('Ship it after the notes', [first]);
  const doing = await mk('Already being done');
  await client.tool('task_claim', { task_id: doing });

  await page.getByRole('tab', { name: /^(Shared tasks and files|Ortak görevler ve dosyalar)$/ }).click();
  await page.getByRole('button', { name: /^(Open board|Panoyu aç)/ }).click();
  const board = page.getByRole('dialog');
  await board.waitFor();
  const col = name => board.locator(`[data-column="${name}"]`);
  await col('backlog').getByText('Write the release notes', { exact: true }).waitFor();
  // The action is on Backlog cards only, and each names its task for a screen reader.
  assert.equal(await col('backlog').getByRole('button', { name: /to an agent$/ }).count(), 3);
  assert.equal(await col('doing').getByRole('button', { name: /to an agent$/ }).count(), 0);

  const screenshots = process.env.AGORA_SCREENSHOTS;
  const shoot = async (name) => {
    if (!screenshots) return;
    mkdirSync(screenshots, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme: theme });
      // The page follows the system scheme through `<html data-theme>`; buttons fade their colours (150 ms). Shoot only once the page
      // shows the scheme asked for and the fade is over, or a light page can carry the dark buttons.
      await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme);
      await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
      await page.waitForTimeout(300);
      await page.screenshot({ path: join(screenshots, `board-give-${name}-${theme}.png`), animations: 'disabled' });
    }
  };

  // Keyboard path: focus the card's action, Enter opens the picker.
  await col('backlog').getByRole('button', { name: 'Give “Write the release notes” to an agent' }).focus();
  await page.keyboard.press('Enter');
  const give = page.locator('[data-give-dialog]');
  await give.waitFor();
  // The picker lists the project's chat agent; the plain terminal is not an agent and is not offered.
  const rows = give.locator('[data-agent-id]');
  await rows.first().waitFor();
  assert.deepEqual(await rows.evaluateAll(els => els.map(e => e.getAttribute('data-agent-id').split('/')[1])), [chat]);
  const height = await rows.first().locator('button').evaluate(b => b.getBoundingClientRect().height);
  assert.ok(height >= 44, `picker row is ${height}px high`);
  await shoot('picker');
  await rows.first().locator('button').click();
  await give.locator('[data-give-step="confirm"]').waitFor();
  const confirm = await give.innerText();
  for (const part of ['Write the release notes', 'fake-adapter', 'Give this task to']) assert.ok(confirm.includes(part), `confirmation names ${part}: ${confirm}`);
  // The machine: the harness names it `laptop` through HOSTNAME, which Windows does not use for the machine name (it shows the computer's
  // own name there), so either is accepted.
  const machines = ['laptop', process.env.COMPUTERNAME, hostname()].filter(Boolean).map((m) => m.toLowerCase());
  assert.ok(machines.some((m) => confirm.toLowerCase().includes(m)), `confirmation names the machine (${machines.join(' or ')}): ${confirm}`);
  assert.doesNotMatch(confirm, /\b(claim|lease|mesh|dispatch)/i); // "release" is not "lease"
  await shoot('confirm');
  // The focus is on Back, never on "Give the task": a fresh Tab and Enter send it.
  assert.equal(await page.evaluate(() => document.activeElement?.textContent), 'Back');
  await page.keyboard.press('Tab');
  await page.keyboard.press('Enter');
  await give.locator('[data-give-step="result"]').waitFor();
  const answer = await give.locator('[role=status]').innerText();
  assert.match(answer, /^Sent to .+\.( It waits until the agent's current turn ends\.)?$/);
  await shoot('sent');
  await give.getByRole('button', { name: 'Done', exact: true }).click();
  await give.waitFor({ state: 'detached' });
  // The node took the task for that agent: it moves to Doing.
  await col('doing').getByText('Write the release notes', { exact: true }).waitFor({ timeout: 20000 });

  // A task that waits for another one is refused with its sentence (the blocking task is not done yet).
  await col('backlog').getByRole('button', { name: 'Give “Ship it after the notes” to an agent' }).click();
  await give.locator('[data-agent-id] button').first().click();
  await give.getByRole('button', { name: 'Give the task', exact: true }).click();
  await give.locator('[role=alert]').waitFor();
  assert.equal(await give.locator('[role=alert]').innerText(), 'This task waits for another task to be done first.');
  await shoot('refused');
  await page.keyboard.press('Escape');
  await give.waitFor({ state: 'detached' });
  await page.keyboard.press('Escape');
  await board.waitFor({ state: 'detached' });

  // The command palette finds a Backlog task by its title and opens the same dialog.
  await page.keyboard.press('Control+k');
  await page.keyboard.type('Port the parser');
  const item = page.getByRole('option', { name: 'Give to an agent: Port the parser to the new API' });
  await item.waitFor();
  // Holding Enter: the first press opens the dialog; its repeats must neither pick an agent nor send.
  await page.keyboard.down('Enter');
  await give.waitFor();
  for (let i = 0; i < 4; i++) await page.keyboard.down('Enter'); // already down: sent with repeat = true
  await page.keyboard.up('Enter');
  assert.equal(await give.locator('[data-give-step]').count(), 0, 'a held Enter stays on the picker');
  assert.ok((await give.innerText()).includes('Port the parser to the new API'));
  if (screenshots) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.emulateMedia({ colorScheme: 'light' });
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'light');
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(screenshots, 'board-give-phone-light.png'), animations: 'disabled' });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  await page.keyboard.press('Escape');
  await give.waitFor({ state: 'detached' });
  assert.deepEqual(errors, []);
  console.log('PASS: a Backlog task is given to the project\'s chat agent from its card (keyboard) and found in the palette; a blocked one is refused with its sentence.');
} finally {
  client?.close();
  await stack?.close();
}
