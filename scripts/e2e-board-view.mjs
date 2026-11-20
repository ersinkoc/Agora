// ADR-0095 step 1: the read-only board view (Backlog, Doing, Done) over real mesh tasks, with disposable shell agents.
// No live hub or real coding agent/account is used. Needs the normal browser-harness prerequisites.
// AGORA_SCREENSHOTS=<dir> also saves screenshots in both themes and at phone width.
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
  await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'board-view-fixture', version: '1' } });
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
const clients = [];
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page, run, bin, env, home, errors } = stack;
  page.setDefaultTimeout(12000);
  const ids = [];
  for (let i = 0; i < 2; i++) {
    const out = run(['agent', 'start', 'shell']);
    const id = out.match(/agent-\d+/)?.[0];
    assert.ok(id, out);
    ids.push(id);
  }
  await page.waitForFunction(() => document.querySelectorAll('aside button[aria-pressed]').length >= 2);
  for (const id of ids) clients.push(await mesh(bin, env, join(home, 'm', `${id}.sock`)));
  const [a, b] = clients;
  const mk = async (c, title, detail) => (await c.tool('task_create', { title, detail, depends_on: [] })).task_id;
  await mk(a, 'Write the release notes', 'plain detail');
  await mk(a, '<img src=x onerror=alert(1)> [click me](https://evil.example) <b>bold</b>');
  await mk(a, 'A very long title '.repeat(40));
  const doing = await mk(b, 'Port the parser to the new API');
  await b.tool('task_claim', { task_id: doing });
  await b.tool('lease_acquire', { paths: ['src/parser.rs', 'src/lexer.rs'], ttl_s: 300, reason: 'fixture' });
  const done = await mk(a, 'Fix the flaky test');
  await a.tool('task_claim', { task_id: done });
  await a.tool('task_complete', { task_id: done, summary: 'fixed' });

  await page.getByRole('tab', { name: /^(Shared tasks and files|Ortak görevler ve dosyalar)$/ }).click();
  await page.getByRole('button', { name: /^(Open board|Panoyu aç)/ }).click();
  const dialog = page.getByRole('dialog');
  await dialog.waitFor();
  const col = id => dialog.locator(`[data-column="${id}"]`);
  await col('backlog').getByText('Write the release notes', { exact: true }).waitFor();
  await col('doing').getByText('Port the parser to the new API').waitFor();
  await col('done').getByText('Fix the flaky test').waitFor();
  assert.equal(await dialog.locator('img, a').count(), 0, 'task text never becomes markup');
  assert.ok((await col('backlog').innerText()).includes('<img src=x'));
  assert.ok((await col('doing').innerText()).includes('src/parser.rs'));

  const screenshots = process.env.AGORA_SCREENSHOTS;
  if (screenshots) {
    mkdirSync(screenshots, { recursive: true });
    const shot = (name) => page.screenshot({ path: join(screenshots, name), animations: 'disabled' });
    for (const theme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme: theme });
      await shot(`board-view-${theme}.png`);
    }
    await page.setViewportSize({ width: 760, height: 844 });
    await shot('board-view-narrow-light.png');
    await page.evaluate(() => { document.documentElement.style.zoom = '1.5'; });
    await page.setViewportSize({ width: 1024, height: 768 });
    await shot('board-view-zoom150-light.png');
    await page.evaluate(() => { document.documentElement.style.zoom = ''; });
    await page.setViewportSize({ width: 760, height: 844 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'detached' });
  assert.deepEqual(errors, []);
  console.log('PASS: the board view shows Backlog, Doing and Done over real tasks, with task text as plain text and held paths on the claimed card.');
} finally {
  for (const c of clients) c.close();
  await stack?.close();
}
