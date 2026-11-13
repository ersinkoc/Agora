// ADR-0058 in a real browser: an ACP agent (agora-fake-agent --acp through the bridge) offers modes and models; a mode that skips
// permissions is marked, never preselected and needs a confirmation that is bound to its agent; a refused switch is said; notices
// for an empty, a failed and an exited turn carry no secret; context and cost are shown; desktop in both themes and a phone fit.
// Only agora-fake-agent is used. See browser-harness.mjs for prerequisites.
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
  const id = out => out.match(/agent-\d+/)?.[0];
  const a = id(run(['agent', 'start', 'fake-acp-a', '--cwd', proj]));
  const b = id(run(['agent', 'start', 'fake-acp-b', '--cwd', proj]));
  const open = async agent => {
    await page.goto(`${origin}/#/agent/${encodeURIComponent(`laptop/${agent}`)}`);
    await page.waitForSelector('form textarea', { timeout: 30000 });
  };
  const mode = page.getByRole('combobox', { name: 'Mode', exact: true });
  const model = page.getByRole('combobox', { name: 'Model', exact: true });
  const confirmation = page.getByText('Let the agent act without asking?', { exact: true });
  const say = async text => { await page.fill('form textarea', text); await page.click('button:has-text("Send")'); };
  const conversation = () => page.locator('[aria-live][aria-label="Conversation"]').innerText().catch(() => '');

  // 1. Pickers: the skipping mode is marked and not preselected.
  await open(a);
  await mode.waitFor({ timeout: 30000 });
  const modes = await mode.locator('option').allTextContents();
  const bypass = modes.find(m => /does not ask/.test(m));
  check(!!bypass, `the permission-skipping mode is marked: ${JSON.stringify(modes)}`);
  check(!/does not ask/.test(await mode.locator('option:checked').innerText()), 'it is not preselected');
  check((await model.locator('option').count()) >= 2, `the models are offered: ${JSON.stringify(await model.locator('option').allTextContents())}`);

  // 2. Confirmation: Keep asking leaves the mode; Switch anyway switches.
  const before = await mode.inputValue();
  await mode.selectOption({ label: bypass });
  await confirmation.waitFor();
  check(true, 'choosing it asks for confirmation first');
  await page.getByRole('button', { name: 'Keep asking', exact: true }).click();
  await page.waitForTimeout(800);
  check((await mode.inputValue()) === before, '"Keep asking" leaves the mode as it was');
  await mode.selectOption({ label: bypass });
  await confirmation.waitFor();
  await page.getByRole('button', { name: 'Switch anyway', exact: true }).click();
  const until = async (ok, ms = 10000) => { const end = Date.now() + ms; while (Date.now() < end) { if (await ok()) return true; await page.waitForTimeout(250); } return false; };
  check(await until(async () => /does not ask/.test(await mode.locator('option:checked').innerText())), '"Switch anyway" switches it');
  await mode.selectOption('plan');
  check(await until(async () => (await mode.inputValue()) === 'plan' && (await confirmation.count()) === 0), 'switching back to an ordinary mode needs no confirmation');

  // 3. The confirmation belongs to its agent: opened on A, the focus moves to B, nothing is sent to B (#418).
  await mode.selectOption({ label: bypass });
  await confirmation.waitFor();
  await open(b);
  await mode.waitFor({ timeout: 30000 });
  check((await confirmation.count()) === 0, 'moving to another agent closes the open confirmation');
  check(!/does not ask/.test(await mode.locator('option:checked').innerText()), 'and the other agent\'s mode is unchanged');

  // 4. A refused switch is said, and the model stays.
  await open(a);
  await model.waitFor({ timeout: 30000 });
  const modelBefore = await model.inputValue();
  await model.selectOption({ index: 1 });
  const refused = await page.getByText('The agent did not switch.', { exact: true }).waitFor({ timeout: 10000 }).then(() => true, () => false);
  check(refused, 'a model the agent refuses is reported');
  check((await model.inputValue()) === modelBefore, `and the model shown stays ${modelBefore}`);

  // 5. Context and cost, then notices: empty, failed, exited. No key, no home folder.
  await say('usage');
  await page.getByText(/reported by the agent/).first().waitFor({ timeout: 15000 }).catch(() => {});
  const bar = await page.locator('main').innerText();
  check(/Context \S+%/.test(bar), `the context fill is shown: ${(bar.match(/Context \S+%/) ?? [''])[0]}`);
  check(/reported by the agent/.test(bar), 'the cost is shown');
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
