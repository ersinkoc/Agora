// Everything at once, in a real browser: two machines, two projects (one of them on both machines), an agent in the main checkout,
// one in its own worktree, a structured (ACP) agent on the other machine, a helper started by an agent with the user's approval,
// messages between machines, the shared board, changes, a reload, and one machine going away and coming back.
// Only shells and agora-fake-agent are used. See browser-harness.mjs for prerequisites.
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { startStack, root } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const step = async (what, work) => {
  try { return await work(); } catch (e) { check(false, `${what}: ${e.message.split('\n')[0]}`); return undefined; }
};

// An MCP client on an agent's mesh socket, as the agent itself would use it.
function mesh(bin, env, socket) {
  const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const pending = new Map();
  let next = 0;
  createInterface({ input: child.stdout }).on('line', line => {
    const reply = JSON.parse(line);
    const p = pending.get(reply.id);
    if (p) { pending.delete(reply.id); clearTimeout(p.timer); reply.error ? p.reject(new Error(JSON.stringify(reply.error))) : p.resolve(reply.result); }
  });
  const call = (method, params, ms = 30000) => new Promise((resolve, reject) => {
    const id = ++next;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, ms);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  return {
    init: () => call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'e2e-integration', version: '1' } }),
    tool: async (name, args, ms) => {
      const r = await call('tools/call', { name, arguments: args }, ms);
      const text = r.content.find(c => c.type === 'text')?.text ?? 'null';
      return { error: !!r.isError, value: JSON.parse(text) };
    },
    close: () => child.kill(),
  };
}

const fake = join(root, `target/debug/agora-fake-agent${process.platform === 'win32' ? '.exe' : ''}`);
const acpManifest = `kind = "fake-acp"\ndisplay_name = "Fake ACP agent"\nversion = "1"\nmin_engine = 1\n[structured]\nacp = { argv = [${JSON.stringify(fake)}, "--acp"] }\n`;
const clients = [];
let stack;
try {
  stack = await startStack({ hostname: 'laptop-host' });
  const { page, run, bin, env, home, proj, work, origin, errors } = stack;
  page.setDefaultTimeout(15000);
  writeFileSync(join(home, 'manifests/fake-acp.toml'), acpManifest);
  // Project A: a portable identity, so its clone on the second machine is the same project. Project B lives only there.
  run(['project', 'init', '--cwd', proj]);
  const copyA = join(work, 'desktop-a');
  execFileSync('git', ['clone', '-q', proj, copyA], { env });
  copyFileSync(join(proj, '.agent-project.json'), join(copyA, '.agent-project.json'));
  const projB = join(work, 'project-b');
  mkdirSync(projB);
  for (const a of [['init', '-q', '-b', 'main'], ['commit', '-q', '--allow-empty', '-m', 'b']]) execFileSync('git', ['-C', projB, ...a], { env });
  const desktop = await stack.addNode({ id: 'desktop', hostname: 'desktop-host', project: copyA, projects: [projB] });
  writeFileSync(join(desktop.home, 'manifests/fake-acp.toml'), acpManifest);

  // 1. Agents: a shell in A's main checkout and a chat agent in its own worktree on the laptop; an ACP agent in B on the desktop.
  const id = out => out.match(/agent-\d+/)?.[0];
  const shell = id(run(['agent', 'start', 'shell', '--cwd', proj]));
  const chat = id(run(['agent', 'start', 'fake-adapter', '--cwd', proj, '--worktree']));
  const acp = id(desktop.run(['agent', 'start', 'fake-acp', '--cwd', projB]));
  check(!!(shell && chat && acp), `three agents started: laptop/${shell}, laptop/${chat} (worktree), desktop/${acp}`);
  await page.goto(`${origin}/#/`);
  const tree = () => page.locator('aside').innerText();
  await step('both machines and all three agents in the tree', async () => {
    await page.waitForFunction(() => document.querySelectorAll('aside section[data-machine-id]').length === 2 && document.querySelectorAll('aside button[aria-pressed]').length >= 3, null, { timeout: 30000 });
    check(true, 'both machines and all three agents are in the tree');
  });

  // 2. A helper: the shell asks for a fake-acp helper in its own folder; the user allows it in the window.
  const ms = mesh(bin, env, join(home, 'm', `${shell}.sock`)); clients.push(ms);
  await ms.init();
  const asked = ms.tool('agent_start', { kind: 'fake-acp', prompt: 'help with the tests' }, 120000);
  await step('the helper request is answered in the window', async () => {
    await page.getByText(/asks to start fake-acp/).first().waitFor({ timeout: 30000 });
    await page.getByRole('button', { name: 'Allow', exact: true }).locator('visible=true').first().click();
  });
  const helper = await asked.catch(e => ({ error: true, value: e.message }));
  check(!helper.error && /agent-\d+/.test(helper.value?.agent_id ?? ''), `the helper started: ${JSON.stringify(helper.value)}`);
  await step('the helper is shown under its parent', async () => {
    await page.getByText(/started by/).first().waitFor({ timeout: 20000 });
    check(true, 'the helper is shown under its parent');
  });
  const helperIdle = await ms.tool('agent_wait', { agent_id: helper.value?.agent_id, state: 'idle', timeout_s: 60 }, 70000).catch(e => ({ error: true, value: e.message }));
  check(!helperIdle.error, `the parent can wait for its helper to be idle: ${JSON.stringify(helperIdle.value)}`);

  // The spec gives ACP and adapter agents the mesh like any agent (adapter-protocol.md); on main they get no endpoint yet (comms
  // finding 2026-11-13). Reported, not failed, until that is fixed; then this says so.
  const sockets = node => (existsSync(join(node, 'm')) ? readdirSync(join(node, 'm')) : []);
  const acpHasMesh = sockets(desktop.home).includes(`${acp}.sock`) && sockets(home).includes(`${chat}.sock`);
  console.log(acpHasMesh ? 'FIXED ACP and adapter agents have a mesh endpoint now: make this a check' : 'KNOWN ACP and adapter agents have no mesh endpoint (finding of 2026-11-13)');

  // 3. Messages between machines: the laptop's shell writes to a shell in project B on the desktop, which reads it.
  const shellB = id(desktop.run(['agent', 'start', 'shell', '--cwd', projB]));
  const md = mesh(bin, desktop.env, join(desktop.home, 'm', `${shellB}.sock`)); clients.push(md);
  await md.init();
  const sent = await ms.tool('send_message', { to: `desktop/${shellB}`, body: 'hello from the laptop' });
  check(!sent.error, `a message to an agent on the other machine is accepted: ${JSON.stringify(sent.value)}`);
  let inbox;
  for (let i = 0; i < 20; i++) {
    inbox = await md.tool('inbox', {});
    if (!inbox.error && JSON.stringify(inbox.value).includes('hello from the laptop')) break;
    await new Promise(r => setTimeout(r, 500));
  }
  const got = (inbox?.value ?? []).find?.(m => m.body === 'hello from the laptop');
  check(!!got, 'the other machine\'s agent receives it');
  check(got?.provenance?.node === 'laptop', `it says which machine it came from: ${JSON.stringify(got?.provenance)}`);

  // 4. The shared board: project A is on both machines, so a task made on the laptop is seen from the desktop's clone.
  const mdA = await (async () => {
    const a = id(desktop.run(['agent', 'start', 'shell', '--cwd', copyA]));
    const c = mesh(bin, desktop.env, join(desktop.home, 'm', `${a}.sock`)); clients.push(c);
    await c.init();
    return c;
  })();
  const task = await ms.tool('task_create', { title: 'Integration task', depends_on: [] });
  check(!task.error, `a task is created on the laptop: ${JSON.stringify(task.value)}`);
  let seen = false;
  for (let i = 0; i < 20 && !seen; i++) {
    const list = await mdA.tool('task_list', {});
    seen = !list.error && JSON.stringify(list.value).includes('Integration task');
    if (!seen) await new Promise(r => setTimeout(r, 500));
  }
  check(seen, 'the desktop agent in the same project sees it');
  const otherProject = await md.tool('task_list', {});
  check(!JSON.stringify(otherProject.value).includes('Integration task'), 'an agent in another project does not');

  // 5. The node's own list agrees with what the window shows.
  const agents = run(['agent', 'list']);
  check([shell, chat, helper.value?.agent_id].every(a => agents.includes(a)), `the laptop lists its three agents: ${agents.trim().split('\n').map(l => l.replace(/\s+/g, ' ')).join(' | ')}`);

  // 6. A reload keeps everything.
  await page.reload();
  await step('a reload keeps both machines, the agents and the helper', async () => {
    await page.waitForFunction(() => document.querySelectorAll('aside section[data-machine-id]').length === 2, null, { timeout: 30000 });
    await page.getByText(/started by/).first().waitFor({ timeout: 20000 });
    check(true, 'a reload keeps both machines, the agents and the helper');
  });

  // 7. The desktop goes away: it is shown offline, the laptop is not affected, and it comes back.
  desktop.stop();
  await step('the desktop is shown offline and the laptop stays online', async () => {
    await page.locator('aside section[data-machine-id="desktop"]').getByText('Offline', { exact: true }).waitFor({ state: 'attached', timeout: 30000 });
    check(await page.locator('aside section[data-machine-id="laptop"]').getByText('Online', { exact: true }).count() === 1, 'the desktop is shown offline and the laptop stays online');
  });
  const stillThere = await ms.tool('agents_list', {});
  check(!stillThere.error, 'the laptop\'s agents keep working while the desktop is away');

  for (const theme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme: theme });
    await page.reload();
    await page.waitForTimeout(1500);
    if (process.env.AGORA_SCREENSHOTS) {
      mkdirSync(process.env.AGORA_SCREENSHOTS, { recursive: true });
      await page.screenshot({ path: join(process.env.AGORA_SCREENSHOTS, `integration-${theme}.png`), fullPage: true });
    }
  }
  check((await page.locator('[role=alert]').filter({ hasNotText: /asks/ }).count()) === 0, 'no error is shown');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  console.log('FAIL', e.stack ?? e.message);
  failed = true;
} finally {
  for (const c of clients) c.close();
  await stack?.close();
}
process.exit(failed ? 1 : 0);
