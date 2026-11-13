// The hub goes away and comes back, in a real browser: two machines, a shell and a chat agent on the first. The hub process is
// killed (not stopped politely) and, after HUB_DOWN_MS (default 15 s), started again on the same data and port. While it is down
// the first machine is used on its own: its agents keep running, agents start and stop, a shell is typed into, and messages between
// its own agents are delivered, while a message to the other machine comes back to its sender as not delivered. After the hub is
// back the node and the window reconnect by themselves, the window is still signed in and shows what happened meanwhile.
// The reconnect times are printed, not checked. Only shells and agora-fake-agent are used. See browser-harness.mjs for prerequisites.
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const step = async (what, work) => {
  try { return await work(); } catch (e) { check(false, `${what}: ${e.message.split('\n')[0]}`); return undefined; }
};
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));

// One tool call on an agent's mesh socket, as the agent itself would make it; a call that does not answer in time is reported as
// such instead of hanging the script.
function meshCall(bin, env, socket, name, args, ms = 15000) {
  return new Promise(resolve => {
    const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
    let done = false;
    const finish = value => { if (done) return; done = true; clearTimeout(timer); child.kill(); resolve(value); };
    const timer = setTimeout(() => finish({ hang: true }), ms);
    const send = message => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n');
    createInterface({ input: child.stdout }).on('line', line => {
      const reply = JSON.parse(line);
      if (reply.id === 1) {
        send({ method: 'notifications/initialized' });
        send({ id: 2, method: 'tools/call', params: { name, arguments: args } });
      } else if (reply.id === 2) {
        if (reply.error) { finish({ error: JSON.stringify(reply.error) }); return; }
        const text = reply.result.content.find(c => c.type === 'text')?.text ?? 'null';
        finish({ isError: !!reply.result.isError, value: JSON.parse(text) });
      }
    });
    child.on('exit', code => finish({ error: `the mesh client exited (${code})` }));
    send({ id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'e2e-link-resilience', version: '1' } } });
  });
}
// A mesh client that stays open, for polling without starting a process per call (the reconnect times are measured with it).
function meshClient(bin, env, socket) {
  const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const pending = new Map();
  let next = 1;
  createInterface({ input: child.stdout }).on('line', line => {
    const reply = JSON.parse(line);
    pending.get(reply.id)?.(reply);
    pending.delete(reply.id);
  });
  const call = (method, params) => new Promise(resolve => {
    const id = ++next;
    pending.set(id, resolve);
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  const ready = call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'e2e-link-resilience', version: '1' } })
    .then(() => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n'));
  return {
    tool: async (name, args) => {
      await ready;
      const reply = await call('tools/call', { name, arguments: args });
      const text = reply.result?.content?.find(c => c.type === 'text')?.text ?? 'null';
      return JSON.parse(text);
    },
    close: () => child.kill(),
  };
}
const sent = r =>!!r && !r.hang && !r.error && !r.isError && typeof r.value?.message_id === 'string';

let stack;
try {
  stack = await startStack({ hostname: 'laptop-host' });
  const { page, run, bin, env, home, proj, work, origin, errors } = stack;
  page.setDefaultTimeout(15000);
  const id = out => out.match(/agent-\d+/)?.[0];
  const list = () => run(['agent', 'list']).trim().split('\n').map(line => line.split('\t'));
  const laptop = (agent, name, args, ms) => meshCall(bin, env, join(home, 'm', `${agent}.sock`), name, args, ms);

  // A portable project on both machines, so the desktop's agent is reachable from the laptop's.
  run(['project', 'init', '--cwd', proj]);
  const clone = join(work, 'desktop-proj');
  execFileSync('git', ['clone', '-q', proj, clone], { env });
  copyFileSync(join(proj, '.agent-project.json'), join(clone, '.agent-project.json'));
  const desktop = await stack.addNode({ id: 'desktop', hostname: 'desktop-host', project: clone });
  const shell = id(run(['agent', 'start', 'shell', '--cwd', proj]));
  const chat = id(run(['agent', 'start', 'fake-adapter', '--cwd', proj]));
  const doomed = id(run(['agent', 'start', 'shell', '--cwd', proj]));
  const remote = id(desktop.run(['agent', 'start', 'fake-adapter', '--cwd', clone]));
  check(!!(shell && chat && doomed && remote), `four agents started: laptop/${shell}, laptop/${chat}, laptop/${doomed}, desktop/${remote}`);
  await step('the window shows both machines', () => page.goto(`${origin}/#/`).then(() =>
    page.waitForFunction(() => document.querySelectorAll('aside section[data-machine-id]').length === 2, null, { timeout: 30000 })));
  check(sent(await laptop(shell, 'send_message', { to: chat, body: 'before, same machine' })), 'a message on the laptop is delivered before the outage');
  const deskBodies = async () => { const r = await meshCall(bin, desktop.env, join(desktop.home, 'm', `${remote}.sock`), 'inbox', {}); return Array.isArray(r.value) ? r.value.map(m => m.body) : []; };
  check(sent(await laptop(shell, 'send_message', { to: `desktop/${remote}`, body: 'before, other machine' })), 'a message to the desktop is accepted before the outage');
  let arrived = false;
  for (let i = 0; i < 20 && !arrived; i++) { await wait(500); arrived = (await deskBodies()).includes('before, other machine'); }
  check(arrived, 'and it is in the desktop agent\'s inbox');

  // 1. The hub is killed.
  await stack.stopHub();
  const down = Date.now();
  await step('the window says the hub cannot be reached', () => page.getByText('Hub unreachable').first().waitFor({ timeout: 20000 }));

  // 2. The laptop on its own.
  check(list().filter(r => r[3] === 'running').length === 3, 'the laptop still runs its three agents');
  const late = await step('an agent starts while the hub is down', async () => id(run(['agent', 'start', 'shell', '--cwd', proj])));
  check(!!late, `an agent starts while the hub is down (${late})`);
  check(await step('stop', async () => { run(['agent', 'stop', doomed]); return true; }) === true, `an agent stops while the hub is down (${doomed})`);
  check(await step('prompt', async () => { run(['agent', 'prompt', shell, 'echo TYPED-DURING-OUTAGE']); return true; }) === true, 'a shell is typed into while the hub is down');
  check(sent(await laptop(shell, 'send_message', { to: chat, body: 'during, same machine' })), 'a message between the laptop\'s own agents is delivered while the hub is down');
  const away = await laptop(shell, 'send_message', { to: `desktop/${remote}`, body: 'during, other machine' });
  check(!away.hang && !away.error, 'a message to the desktop does not hang or fail the call while the hub is down');
  let notice;
  for (let i = 0; i < 20 && !notice; i++) {
    await wait(500);
    const inbox = await laptop(shell, 'inbox', {});
    notice = (Array.isArray(inbox.value) ? inbox.value : []).find(m => m.kind === 'delivery_failed');
  }
  check(notice?.body === 'node.offline', `its sender is told it was not delivered (${notice ? `${notice.kind}: ${notice.body}` : 'no notice'})`);

  // 3. The hub comes back on the same data and port.
  await wait(Math.max(0, Number(process.env.HUB_DOWN_MS ?? 15000) - (Date.now() - down)));
  const sender = meshClient(bin, env, join(home, 'm', `${shell}.sock`));
  const receiver = meshClient(bin, desktop.env, join(desktop.home, 'm', `${remote}.sock`));
  await sender.tool('agents_list', {});
  await receiver.tool('agents_list', {});
  stack.startHub();
  const up = Date.now();
  // The window is watched from the same moment, on its own, so its time does not include the node's.
  const windowBack = page.getByText('Hub unreachable').first().waitFor({ state: 'detached', timeout: 60000 }).then(() => Date.now() - up, () => 0);
  // `send_message` answers with an id at once and reports a failed delivery later, in the sender's inbox: "back" is when a message
  // sent now is in the desktop's inbox.
  let nodeBack = 0;
  for (let i = 0; Date.now() - up < 60000 && !nodeBack; i++) {
    await sender.tool('send_message', { to: `desktop/${remote}`, body: `after, other machine #${i}` });
    await wait(300);
    const inbox = await receiver.tool('inbox', {});
    if (Array.isArray(inbox) && inbox.some(m => m.body.startsWith('after, other machine'))) nodeBack = Date.now() - up;
  }
  sender.close();
  receiver.close();
  check(nodeBack > 0, 'a message from the laptop reaches the desktop again after the hub is back');
  const windowMs = await windowBack;
  check(windowMs > 0, 'the window reconnects by itself');
  console.log(`     the hub was down ${Math.round((up - down) / 1000)} s; back after it started: the laptop reaches the desktop in ${nodeBack} ms, the window in ${windowMs} ms`);

  // 4. What the window shows afterwards, after a reload too.
  await page.reload();
  check(await page.waitForSelector('aside section[data-machine-id="laptop"]', { timeout: 30000 }).then(() => true, () => false), 'the window is still signed in after the hub restart');
  await step('the window counts the laptop\'s running agents as the laptop does', async () => {
    const running = list().filter(r => r[3] === 'running').length;
    await page.waitForFunction(n => {
      const section = document.querySelector('aside section[data-machine-id="laptop"]');
      return !!section && section.querySelectorAll('button[aria-pressed]').length === n;
    }, running, { timeout: 30000 });
  });
  await step('the agent started during the outage opens', async () => {
    await page.goto(`${origin}/#/agent/${encodeURIComponent(`laptop/${late}`)}`);
    await page.waitForSelector('.xterm-rows');
  });
  await step('the agent stopped during the outage is shown as finished', async () => {
    await page.goto(`${origin}/#/agent/${encodeURIComponent(`laptop/${doomed}`)}`);
    await page.getByText('Finished').first().waitFor();
  });
  await step('the shell shows what was typed during the outage', async () => {
    await page.goto(`${origin}/#/agent/${encodeURIComponent(`laptop/${shell}`)}`);
    await page.waitForFunction(() => /TYPED-DURING-OUTAGE/.test(document.querySelector('.xterm-rows')?.textContent ?? ''), null, { timeout: 20000 });
  });
  const bodies = await deskBodies();
  check(bodies.includes('before, other machine') && !bodies.includes('during, other machine'), `the desktop keeps the message from before and has none of the undelivered one (${bodies.join(' | ')})`);
  const local = await laptop(chat, 'inbox', {});
  const localBodies = (Array.isArray(local.value) ? local.value : []).map(m => m.body);
  check(['before, same machine', 'during, same machine'].every(b => localBodies.includes(b)), 'the laptop\'s chat agent has its messages from before and during');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  check(false, e.message.split('\n')[0]);
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
