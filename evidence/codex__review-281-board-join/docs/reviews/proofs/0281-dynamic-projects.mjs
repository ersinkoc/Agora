// Review proof: two projects selected after both nodes start must share tasks and exclude conflicting leases.
// Requires the normal scripts/browser-harness.mjs prerequisites; uses disposable shells, never real agents/accounts.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { startStack } from '../../../scripts/browser-harness.mjs';

async function mesh(bin, env, socket) {
  const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const pending = new Map();
  let next = 0;
  createInterface({ input: child.stdout }).on('line', line => {
    const reply = JSON.parse(line);
    const request = pending.get(reply.id);
    if (!request) return;
    pending.delete(reply.id); clearTimeout(request.timer);
    if (reply.error) request.reject(new Error(JSON.stringify(reply.error)));
    else request.resolve(reply.result);
  });
  const call = (method, params) => new Promise((resolve, reject) => {
    const id = ++next;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP timed out: ${method}`)); }, 15000);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'dynamic-project-proof', version: '1' } });
  return {
    tool: async (name, args) => {
      const reply = await call('tools/call', { name, arguments: args });
      return { failed: !!reply.isError, value: JSON.parse(reply.content.find(item => item.type === 'text').text) };
    },
    close: () => child.kill(),
  };
}

const fixture = mkdtempSync(join(tmpdir(), 'dynamic-project-'));
const paths = ['local', 'peer'].map(name => join(fixture, name, 'project'));
const gitEnv = { ...process.env, GIT_AUTHOR_NAME: 'fixture', GIT_AUTHOR_EMAIL: 'fixture@example.com', GIT_COMMITTER_NAME: 'fixture', GIT_COMMITTER_EMAIL: 'fixture@example.com' };
let stack;
const clients = [];
try {
  for (const path of paths) {
    mkdirSync(path, { recursive: true });
    execFileSync('git', ['init', '-q', '-b', 'main', path], { env: gitEnv });
    execFileSync('git', ['-C', path, 'commit', '-q', '--allow-empty', '-m', 'fixture'], { env: gitEnv });
    writeFileSync(join(path, '.agent-project.json'), JSON.stringify({ project_id: '11111111-1111-4111-8111-111111111111' }));
  }
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, web: process.env.AGORA_TEST_WEB });
  // Both nodes start with the harness's unrelated project, not either newly selected folder.
  const peer = await stack.addNode({ id: 'desktop', project: stack.proj });
  const start = (run, path) => {
    const output = run(['agent', 'start', 'shell', '--cwd', path]);
    const id = output.match(/agent-\d+/)?.[0];
    assert.ok(id, output); return id;
  };
  const localId = start(stack.run, paths[0]);
  const peerId = start(peer.run, paths[1]);
  await stack.page.waitForFunction(() => document.querySelectorAll('aside button[aria-pressed]').length === 2);
  clients.push(await mesh(stack.bin, stack.env, join(stack.home, 'm', `${localId}.sock`)));
  clients.push(await mesh(stack.bin, peer.env, join(peer.home, 'm', `${peerId}.sock`)));
  const task = await clients[0].tool('task_create', { title: 'Task made on laptop', depends_on: [] });
  const localLease = await clients[0].tool('lease_acquire', { paths: ['README.md'], ttl_s: 120 });
  assert.equal(task.failed, false); assert.equal(localLease.failed, false);
  const peerTasks = await clients[1].tool('task_list', {});
  const peerLease = await clients[1].tool('lease_acquire', { paths: ['README.md'], ttl_s: 120 });
  console.log(JSON.stringify({ localLease, peerTasks, peerLease }, null, 2));
  assert.equal(peerTasks.failed, false);
  assert.ok(peerTasks.value.some(task => task.title === 'Task made on laptop'), 'Peer lost sight of a task on the same dynamically selected project');
  assert.equal(peerLease.failed, true, 'Two nodes were granted the same project path concurrently');
  assert.equal(peerLease.value.error.code, 'lease.held');
} catch (error) {
  console.error(error.stack || error); process.exitCode = 1;
} finally {
  for (const client of clients) client.close();
  await stack?.close(); rmSync(fixture, { recursive: true, force: true });
}
