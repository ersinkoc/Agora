// #286 / LAN L15: real shared boards for two same-named projects, with two disposable nodes and shell sessions.
// No live hub or real coding agent/account is used. Needs the normal browser-harness prerequisites.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { startStack } from './browser-harness.mjs';

// An MCP client attached only to a shell in this disposable stack.
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
  await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'board-fixture', version: '1' } });
  return {
    tool: async (name, args) => {
      const result = await call('tools/call', { name, arguments: args });
      assert.equal(!!result.isError, false, JSON.stringify(result));
      return JSON.parse(result.content.find(item => item.type === 'text').text);
    },
    close: () => child.kill(),
  };
}

const fixture = mkdtempSync(join(tmpdir(), 'board-'));
const ids = ['11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-222222222222'];
const paths = ['a', 'b', 'copy-a', 'copy-b'].map(name => join(fixture, name, 'shared'));
const gitEnv = { ...process.env, GIT_AUTHOR_NAME: 'fixture', GIT_AUTHOR_EMAIL: 'fixture@example.com', GIT_COMMITTER_NAME: 'fixture', GIT_COMMITTER_EMAIL: 'fixture@example.com' };
let stack;
const clients = [];
try {
  for (let i = 0; i < paths.length; i++) {
    mkdirSync(paths[i], { recursive: true });
    execFileSync('git', ['init', '-q', '-b', 'main', paths[i]], { env: gitEnv });
    execFileSync('git', ['-C', paths[i], 'commit', '-q', '--allow-empty', '-m', 'fixture'], { env: gitEnv });
    writeFileSync(join(paths[i], '.agent-project.json'), JSON.stringify({ project_id: ids[i % 2] }));
  }
  stack = await startStack({ projects: paths.slice(0, 2), hostname: 'Same laptop', binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page, run, bin, env, home, errors } = stack;
  page.setDefaultTimeout(12000);
  const peer = await stack.addNode({ id: 'desktop', hostname: 'Same laptop', project: paths[2], projects: [paths[3]] });
  const start = (run, path) => {
    const output = run(['agent', 'start', 'shell', '--cwd', path]);
    const id = output.match(/agent-\d+/)?.[0];
    assert.ok(id, output);
    return id;
  };
  const local = paths.slice(0, 2).map(path => start(run, path));
  paths.slice(2).forEach(path => start(peer.run, path));
  // Wait for the independent node/agent facts before the first shared-board tool call.
  await page.waitForFunction(() => document.querySelectorAll('aside button[aria-pressed]').length === 4);
  for (const id of local) clients.push(await mesh(bin, env, join(home, 'm', `${id}.sock`)));
  for (let i = 0; i < clients.length; i++) {
    await clients[i].tool('task_create', { title: `Project ${i ? 'B' : 'A'} task`, depends_on: [] });
    await clients[i].tool('lease_acquire', { paths: [`src/${i ? 'b' : 'a'}.txt`], ttl_s: 120, reason: 'fixture' });
  }
  await page.getByText('Project A task', { exact: true }).waitFor();
  await page.getByText('Project B task', { exact: true }).waitFor();
  const cards = ids.map(id => page.locator(`[data-project-id="id:${id}"]`));
  await cards[0].waitFor();
  for (let i = 0; i < cards.length; i++) {
    assert.equal(await cards[i].locator('strong').innerText(), 'shared');
    assert.ok((await cards[i].innerText()).includes(`id:${ids[i]}`));
    await cards[i].locator('summary').click();
    assert.ok((await cards[i].innerText()).includes(paths[i]));
    assert.ok((await cards[i].innerText()).includes(paths[i + 2]));
    assert.ok((await cards[i].innerText()).includes('src/' + (i ? 'b' : 'a') + '.txt'));
    assert.equal(await cards[i].getByText(`Project ${i ? 'A' : 'B'} task`, { exact: true }).count(), 0);
  }
  await cards[1].locator('summary').click();
  await clients[1].tool('task_create', { title: 'Second B task', depends_on: [] });
  await cards[1].getByText('Second B task', { exact: true }).waitFor();
  assert.equal(await cards[0].locator('details').getAttribute('open'), '');
  assert.equal(await cards[1].locator('details').getAttribute('open'), null);
  await page.reload();
  await cards[0].getByText('Project A task', { exact: true }).waitFor();
  await cards[1].getByText('Second B task', { exact: true }).waitFor();
  assert.ok((await cards[0].innerText()).includes('src/a.txt'));
  const screenshots = process.env.AGORA_SCREENSHOTS;
  if (screenshots) {
    mkdirSync(screenshots, { recursive: true });
    await page.setViewportSize({ width: 1440, height: 1100 });
    for (const lang of ['en', 'tr']) {
      if (lang === 'tr') await page.addInitScript(() => { Object.defineProperty(navigator, 'language', { get: () => 'tr-TR' }); });
      for (const theme of ['dark', 'light']) {
        await page.emulateMedia({ colorScheme: theme });
        await page.reload();
        await cards[0].waitFor();
        for (const card of cards) await card.locator('summary').click();
        await page.screenshot({ path: join(screenshots, `boards-${lang}-${theme}.png`), fullPage: true, animations: 'disabled' });
      }
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: join(screenshots, 'boards-tr-phone-light.png'), fullPage: true, animations: 'disabled' });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  }
  assert.deepEqual(errors, []);
  console.log('PASS: same-named boards keep separate project IDs, tasks, selected-folder leases and two-machine contexts; details survive updates, reload restores both boards, and both languages/themes fit.');
} catch (error) {
  console.error(error.stack || error);
  if (stack) console.error((await stack.page.locator('body').innerText()).slice(-4500));
  process.exitCode = 1;
} finally {
  for (const client of clients) client.close();
  await stack?.close();
  rmSync(fixture, { recursive: true, force: true });
}
