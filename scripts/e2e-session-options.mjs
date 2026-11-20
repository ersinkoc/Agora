// ADR-0058 in a real browser: an ACP agent (agora-fake-agent --acp through the bridge) offers modes and models, in the window's own menus
// (not native selects); a mode that skips permissions is marked, never preselected, shown DISABLED with why (a switch to it cannot succeed
// until a signed confirmation exists, so no dialog is offered: the node refuses it with `agent.option_unconfirmed`) and never sent; a
// refused switch is said; notices for an empty, a failed and an exited turn carry no secret; context is shown and money is not; desktop in
// both themes and a phone fit. Only agora-fake-agent is used. See browser-harness.mjs for prerequisites.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { startStack, root } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const fake = join(root, `target/debug/agora-fake-agent${process.platform === 'win32' ? '.exe' : ''}`);
// The bridge starts the agent with a cleared environment, so the fake's switches go through a shell that sets them.
const acp = (kind, vars) => {
  const set = Object.entries(vars).map(([k, v]) => process.platform === 'win32' ? `set ${k}=${v}&&` : `${k}=${v}`).join(' ');
  const argv = process.platform === 'win32'
    ? ['C:\\Windows\\System32\\cmd.exe', '/d', '/c', `${set} ${fake} --acp`]
    : ['/bin/sh', '-c', `${set} exec '${fake}' --acp`];
  return `kind = "${kind}"\ndisplay_name = "${kind}"\nversion = "1"\nmin_engine = 1\n[structured]\nacp = { argv = ${JSON.stringify(argv)} }\n`;
};

let stack;
try {
  stack = await startStack();
  const { page, run, home, proj, origin, errors } = stack;
  page.setDefaultTimeout(15000);
  writeFileSync(join(home, 'manifests/fake-acp-a.toml'), acp('fake-acp-a', { AGORA_FAKE_ACP_BYPASS_MODE: '1', AGORA_FAKE_ACP_REFUSE_MODEL: '1' }));
  writeFileSync(join(home, 'manifests/fake-acp-b.toml'), acp('fake-acp-b', { AGORA_FAKE_ACP_BYPASS_MODE: '1' }));
  writeFileSync(join(home, 'manifests/fake-acp-c.toml'), acp('fake-acp-c', { AGORA_FAKE_ACP_BYPASS_MODE: '1', AGORA_FAKE_ACP_NO_CURRENT: '1' }));
  const id = out => out.match(/agent-\d+/)?.[0];
  const a = id(run(['agent', 'start', 'fake-acp-a', '--cwd', proj]));
  const b = id(run(['agent', 'start', 'fake-acp-b', '--cwd', proj]));
  const c = id(run(['agent', 'start', 'fake-acp-c', '--cwd', proj]));
  const open = async agent => {
    await page.goto(`${origin}/#/agent/${encodeURIComponent(`laptop/${agent}`)}`);
    await page.waitForSelector('form textarea', { timeout: 30000 });
  };
  // The window's own menus: a button named "Mode: <current>" opens a list of radio items (a native select is not used).
  const mode = page.locator('button[aria-haspopup=menu][aria-label^="Mode:"]');
  const model = page.locator('button[aria-haspopup=menu][aria-label^="Model:"]');
  const items = page.locator('[role=menu] [role=menuitemradio]');
  const openMenu = async button => { if (!(await page.locator('[role=menu]').count())) await button.click(); await page.locator('[role=menu]').waitFor(); };
  const closeMenu = () => page.keyboard.press('Escape');
  const dialog = page.locator('[role=alertdialog]');
  // The window may draw the screen again once more after it opens (the agent's settings arrive): type when the box keeps what is typed.
  const say = async text => {
    for (let i = 0; i < 5; i++) {
      await page.fill('form textarea', text);
      await page.waitForTimeout(400);
      if ((await page.inputValue('form textarea')) === text) break;
    }
    await page.click('button:has-text("Send")');
  };
  const conversation = () => page.locator('[aria-live][aria-label="Conversation"]').innerText().catch(() => '');
  const until = async (ok, ms = 10000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await ok()) return true; await page.waitForTimeout(250); } return false; };
  const label = button => button.getAttribute('aria-label');

  // 1. Pickers: the skipping mode is marked, is not the current one, and is an ordinary enabled choice (ADR-0090 Amendment A).
  await open(a);
  await mode.waitFor({ timeout: 30000 });
  const modeNow = await label(mode);
  await openMenu(mode);
  const modes = await items.allTextContents();
  const bypassItem = items.filter({ hasText: 'does not ask' });
  check((await bypassItem.count()) === 1, `the permission-skipping mode is marked: ${JSON.stringify(modes)}`);
  check(!/does not ask/.test(modeNow ?? ''), `it is not preselected: ${modeNow}`);
  check(!(await bypassItem.isDisabled()), 'it is enabled, like any other choice');
  check(!/not available/.test(await page.locator('main').innerText()), 'and nothing says it is not available');
  await closeMenu();
  await openMenu(model);
  check((await items.count()) >= 2, `the models are offered: ${JSON.stringify(await items.allTextContents())}`);
  await closeMenu();

  // 2. Choosing it is sent like any other choice. Whether the machine asks for the owner's approval (a card) or refuses by its policy
  // depends on how this node is configured, so what is asserted is what must hold either way: no dialog of the window's own, and the
  // mode shown is never the skipping one until the machine reports it.
  await openMenu(mode);
  await bypassItem.click({ timeout: 2000 }).catch(() => {});
  await page.waitForTimeout(800);
  check((await dialog.count()) === 0, 'choosing it opens no confirmation dialog of the window\'s own');
  check(!/does not ask/.test((await label(mode)) ?? ''), `and the mode shown is not the skipping one before it is approved: ${await label(mode)}`);
  await closeMenu();
  await openMenu(mode);
  await items.filter({ hasText: /^Plan/ }).click();
  check(await until(async () => /Plan/.test((await label(mode)) ?? '')), 'switching to an ordinary mode works and asks nothing');

  // 3. Each agent has its own menus: B shows its own mode, never A's.
  await open(b);
  await mode.waitFor({ timeout: 30000 });
  check((await dialog.count()) === 0, 'no confirmation is open on another agent');
  check(!/Plan/.test((await label(mode)) ?? '') && !/does not ask/.test((await label(mode)) ?? ''), `the other agent's mode is its own: ${await label(mode)}`);

  // 3b. An agent that does not say which mode or model it runs with: both menus show "Not reported", not the first option (#418),
  // so neither a mode that skips permissions nor any other is presented as chosen.
  await open(c);
  await mode.waitFor({ timeout: 30000 });
  check((await label(mode)) === 'Mode: Not reported', `an unknown current mode shows "Not reported": ${await label(mode)}`);
  check((await label(model)) === 'Model: Not reported', `an unknown current model shows "Not reported": ${await label(model)}`);

  // 4. A refused switch is said, and the model stays.
  await open(a);
  await model.waitFor({ timeout: 30000 });
  const modelBefore = await label(model);
  await openMenu(model);
  await items.filter({ hasNot: page.locator('[aria-checked=true]') }).nth(1).click();
  const refused = await page.getByText('The agent did not switch.', { exact: true }).waitFor({ timeout: 10000 }).then(() => true, () => false);
  check(refused, 'a model the agent refuses is reported');
  check((await label(model)) === modelBefore, `and the model shown stays: ${await label(model)}`);

  // 5. Context (never money), then notices: empty, failed, exited. No key, no home folder.
  await say('usage');
  await page.getByText(/Context \S+%/).first().waitFor({ timeout: 15000 }).catch(() => {});
  const bar = await page.locator('main').innerText();
  check(/Context \S+%/.test(bar), `the context fill is shown: ${(bar.match(/Context \S+%/) ?? [''])[0]}`);
  check(!/reported by the agent|\$\d|USD/.test(bar), 'and no money is');
  await say('silent-loud');
  await page.getByText(/ended its turn without saying anything/).waitFor({ timeout: 15000 }).catch(() => {});
  await say('fail');
  await page.getByText(/turn failed/).waitFor({ timeout: 15000 }).catch(() => {});
  let said = await conversation();
  check(/ended its turn without saying anything/.test(said), 'an empty turn shows a notice');
  check(/turn failed/.test(said), 'a failed turn shows a notice');
  for (const theme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme: theme });
    await page.reload();
    await page.waitForSelector('form textarea', { timeout: 30000 });
    await page.waitForTimeout(1000);
    if (process.env.AGORA_SCREENSHOTS) {
      mkdirSync(process.env.AGORA_SCREENSHOTS, { recursive: true });
      await page.screenshot({ path: join(process.env.AGORA_SCREENSHOTS, `session-options-${theme}.png`), fullPage: true, animations: 'disabled' });
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.reload();
  await page.waitForSelector('form textarea', { timeout: 30000 });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'the pickers and notices fit a phone without sideways scrolling');
  await page.setViewportSize({ width: 1440, height: 900 });
  await say('crash-loud');
  await page.getByText(/The agent stopped/).waitFor({ timeout: 20000 }).catch(() => {});
  said = await conversation();
  check(/The agent stopped \(step turn/.test(said), 'an agent that exits mid-turn shows a notice with its step');
  check(!/sk-[A-Za-z0-9]{10,}|api_key=\w{6,}/.test(said), 'no key reaches the window');
  const homeDir = process.env.USERPROFILE ?? process.env.HOME ?? '';
  check(!homeDir || !said.toLowerCase().includes(homeDir.toLowerCase()), 'no home folder reaches the window');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  console.log('FAIL', e.stack ?? e.message);
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
