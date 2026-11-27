// ADR-0109 and ADR-0121: a pending handoff survives a hub process restart and reaches the web board and exactly one inbox entry.
// Requires the browser harness, agora-fake-agent, and a host with SIGSTOP/SIGCONT. The nodes have separate disposable homes.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { cpSync } from 'node:fs';
import { connect } from 'node:net';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { startStack } from './browser-harness.mjs';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function mesh(bin, env, socket) {
  const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  let id = 0;
  const call = async (method, params) => {
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: ++id, method, params }) + '\n');
    const result = AbortSignal.timeout(15_000);
    let cancel;
    try {
      const line = await Promise.race([
        lines.next(),
        new Promise((_, reject) => {
          cancel = () => reject(new Error(`Mesh timed out: ${method}`));
          result.addEventListener('abort', cancel, { once: true });
        }),
      ]);
      assert.ok(!line.done, `Mesh ended before answering ${method}`);
      const reply = JSON.parse(line.value);
      assert.equal(reply.id, id);
      assert.equal(reply.error, undefined, JSON.stringify(reply));
      return reply.result;
    } finally {
      result.removeEventListener('abort', cancel);
    }
  };
  try {
    await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'handoff-fixture', version: '1' } });
    return {
      close: () => child.kill(),
      async tool(name, args = {}) {
        const result = await call('tools/call', { name, arguments: args });
        assert.equal(!!result.isError, false, JSON.stringify(result));
        return JSON.parse(result.content.find(item => item.type === 'text').text);
      },
    };
  } catch (error) {
    child.kill();
    throw error;
  }
}

async function waitForHub(origin) {
  const port = Number(new URL(origin).port);
  for (let i = 0; i < 80; i++) {
    const healthy = await new Promise(resolve => {
      const socket = connect(port, '127.0.0.1');
      const finish = ready => { socket.destroy(); resolve(ready); };
      socket.once('connect', () => finish(true));
      socket.once('error', () => finish(false));
      socket.setTimeout(500, () => finish(false));
    });
    if (healthy) return;
    await sleep(125);
  }
  throw new Error('The restarted hub did not start listening');
}

if (process.platform === 'win32') {
  console.log('SKIP: pausing only the receiver requires SIGSTOP/SIGCONT.');
  process.exit(0);
}

let stack;
let receiver;
const clients = [];
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY });
  const { page, run, bin, env, addNode, home, work, proj, origin, stopHub, startHub, errors } = stack;
  page.setDefaultTimeout(25_000);

  const copy = join(work, 'receiver-project');
  // Both checkouts have the same root commit and no remote, so they have the same project identity.
  cpSync(proj, copy, { recursive: true });
  receiver = await addNode({ id: 'receiver', project: copy });
  const launch = (cli, cwd) => {
    const output = cli(['agent', 'start', 'fake-adapter', '--cwd', cwd]);
    const id = output.match(/agent-\d+/)?.[0];
    assert.ok(id, output);
    return id;
  };
  const from = launch(run, proj);
  const to = launch(receiver.run, copy);
  clients.push(await mesh(bin, env, join(home, 'm', `${from}.sock`)));
  clients.push(await mesh(bin, receiver.env, join(receiver.home, 'm', `${to}.sock`)));
  const [giver, recipient] = clients;

  const title = 'Verify the durable task handoff';
  const summary = 'Read the handoff after the hub comes back';
  // Wait for both agents to agree on the project's shared board before pausing the receiver.
  const task = await giver.tool('task_create', { title, depends_on: [] });
  await giver.tool('task_claim', { task_id: task.task_id });
  assert.ok((await recipient.tool('task_list')).some(row => row.title === title));
  const openCard = async () => {
    const tab = page.getByRole('tab', { name: 'Shared tasks and files', exact: true });
    if (await tab.getAttribute('aria-selected') !== 'true') await tab.click();
    const dialog = page.getByRole('dialog');
    await page.getByRole('button', { name: /^Open board/ }).first().click();
    const card = dialog.locator('[data-task-id]').filter({ hasText: title });
    await card.waitFor();
    return card;
  };
  let card = await openCard();

  assert.ok(receiver.signal('SIGSTOP'), 'pause the receiver before the handoff is sent');
  await giver.tool('handoff', { task_id: task.task_id, to: `receiver/${to}`, summary });
  await card.getByText(/Handoff to .* pending/).waitFor();
  await stopHub();
  startHub();
  await waitForHub(origin);

  // A newly subscribed browser still shows Pending while the receiver cannot answer.
  await page.reload();
  card = await openCard();
  await card.getByText(/Handoff to .* pending/).waitFor();

  assert.ok(receiver.signal('SIGCONT'), 'resume the same receiver process');
  await card.getByText(/Handoff delivered to /).waitFor();
  const inbox = await recipient.tool('inbox');
  assert.equal(inbox.filter(message => message.kind === 'handoff' && message.body === summary).length, 1);
  assert.deepEqual(errors, []);
  console.log('PASS: Pending survived the hub restart and reload. The receiver received one durable handoff and the board showed Delivered.');
} finally {
  receiver?.signal('SIGCONT');
  for (const client of clients) client.close();
  await stack?.close();
}
