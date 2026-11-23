// ADR-0103 (#1020): a window that reloads after agents messaged each other still shows the message in Team chat. Two shells on one
// machine; one sends the other a message over its mesh socket; the window is reloaded (it gets a snapshot) and Team chat must show
// the message. Before ADR-0103 the snapshot had no messages and Team chat was empty after every reload.
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run gen && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-team-chat-reload.mjs
import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };

// An MCP client on an agent's mesh socket, as the agent itself would use it.
function mesh(bin, env, socket) {
  const child = spawn(bin, ['mesh', 'stdio', '--socket', socket], { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const pending = new Map();
  let next = 0;
  createInterface({ input: child.stdout }).on('line', (line) => {
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
    init: () => call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'e2e-team-chat-reload', version: '1' } }),
    tool: async (name, args) => {
      const r = await call('tools/call', { name, arguments: args });
      return { error: !!r.isError, value: r.content.find((c) => c.type === 'text')?.text };
    },
    close: () => child.kill(),
  };
}

const body = 'sent before the reload';
let stack, client;
try {
  stack = await startStack();
  const { page, run, bin, env, home, proj, origin } = stack;
  page.setDefaultTimeout(15000);
  const id = (out) => out.match(/agent-\d+/)?.[0];
  const a = id(run(['agent', 'start', 'shell', '--cwd', proj]));
  const b = id(run(['agent', 'start', 'shell', '--cwd', proj]));
  check(!!(a && b), `two agents started: ${a}, ${b}`);
  await page.goto(`${origin}/#/`);
  await page.waitForFunction(() => document.querySelectorAll('aside button[aria-pressed]').length >= 2, null, { timeout: 30000 });
  client = mesh(bin, env, join(home, 'm', `${a}.sock`));
  await client.init();
  const sent = await client.tool('send_message', { to: b, body });
  check(!sent.error, `the message is accepted: ${sent.value}`);
  const teamChat = async () => {
    // The rail's Team chat icon (its tooltip names it; the icon has no text).
    await page.locator('[data-tip^="Team chat"]').locator('visible=true').first().click();
    return page.getByText(body).first().waitFor({ timeout: 15000 }).then(() => true, () => false);
  };
  check(await teamChat(), 'Team chat shows the message while the window is open');
  for (const round of [1, 2]) {
    await page.reload();
    await page.waitForFunction(() => document.querySelectorAll('aside button[aria-pressed]').length >= 2, null, { timeout: 30000 });
    // Team chat may be open already (remembered on this device): only click when the message is not on screen.
    const shown = (await page.getByText(body).count()) > 0 || (await teamChat());
    check(shown, `Team chat shows the message after reload ${round}`);
    check((await page.getByText(body).count()) === 1, `the message is shown once after reload ${round}`);
  }
  check(stack.errors.length === 0, `no page errors: ${stack.errors.join(' | ')}`);
} catch (e) {
  check(false, e.message.split('\n')[0]);
} finally {
  client?.close();
  await stack?.close();
}
process.exit(failed ? 1 : 0);
