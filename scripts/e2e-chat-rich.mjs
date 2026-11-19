// ADR-0090 in a real browser: the fake ACP agent (agora-fake-agent --acp with config options and commands) through the bridge, the node and the
// hub. Thinking, one row per tool call that changes in place (with a status mark), a plan with step status, the agent's own settings as menus,
// the "/" menu, Stop turn (the agent stays), a queued prompt and dropping the queue, and a permission record that follows the user's answer.
// Only agora-fake-agent is used. See browser-harness.mjs for prerequisites.
import { writeFileSync } from 'node:fs';
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
  page.setDefaultTimeout(20000);
  writeFileSync(join(home, 'manifests/fake-rich.toml'), acp('fake-rich', { AGORA_FAKE_ACP_CONFIG: '1' }));
  const agent = run(['agent', 'start', 'fake-rich', '--cwd', proj]).match(/agent-\d+/)?.[0];
  await page.goto(`${origin}/#/agent/${encodeURIComponent(`laptop/${agent}`)}`);
  await page.waitForSelector('form textarea', { timeout: 30000 });
  // The window may draw the screen again once more after it opens (the agent's settings arrive): type when the box keeps what is typed.
  const say = async text => {
    for (let i = 0; i < 5; i++) {
      await page.fill('form textarea', text);
      await page.waitForTimeout(400);
      if ((await page.inputValue('form textarea')) === text) break;
    }
    await page.click('button:has-text("Send")');
  };
  const stopTurn = page.locator('button[title^="Ends this turn"]');
  const gone = (locator, ms = 15000) => locator.waitFor({ state: 'detached', timeout: ms }).then(() => true, () => false);

  // The agent's settings and commands reach a window that opened after the agent did (the hub replays them after its snapshot).
  await page.waitForSelector('button[aria-label^="Effort"]', { timeout: 30000 });
  check((await page.locator('button[aria-label^="Mode:"]').count()) === 1, 'the mode is one menu, not the older picker as well as the setting');
  await page.fill('form textarea', '/');
  await page.waitForSelector('[role=listbox] [data-slash-option]');
  check((await page.locator('[data-slash-option]').count()) >= 2, 'the "/" menu lists the commands the agent offers');
  await page.keyboard.press('Escape');
  await page.fill('form textarea', '');

  // Choosing an effort sends AgentSetConfig; the agent's answer is the new current value.
  await page.click('button[aria-label^="Effort"]');
  await page.locator('[role=menu] [role=menuitemradio]').filter({ hasNot: page.locator('[aria-checked=true]') }).first().click();
  check(await page.waitForFunction(() => !/Effort: Medium/.test(document.querySelector('button[aria-label^="Effort"]')?.getAttribute('aria-label') ?? ''), null, { timeout: 15000 }).then(() => true, () => false), 'choosing an effort changes it');

  // One turn with thinking, a call that changes in place, and a plan with step status.
  await say('rich');
  await page.waitForSelector('text=done', { timeout: 30000 });
  check((await page.locator('[data-thought]').count()) === 1, 'thinking is one folded block');
  check((await page.locator('[data-steps]').count()) === 1 && (await page.locator('[data-steps] details, [data-steps] [data-status]').count()) === 1, 'a call that changed three times is one row');
  check((await page.locator('[data-steps] [role=img][aria-label="Done"]').count()) === 1, 'and shows a done mark');
  check((await page.locator('[data-plan] [data-status=completed]').count()) === 1 && (await page.locator('[data-plan] [aria-current=step]').count()) === 1, 'the plan ticks a done step and marks the one in hand');

  // A turn that runs: Stop turn is offered; a prompt sent meanwhile waits; dropping the queue asks first.
  await say('slow');
  await stopTurn.waitFor();
  check(true, 'Stop turn is offered while the agent works');
  await say('hold');
  await page.waitForSelector('[data-queue]');
  check(/1 queued/.test(await page.locator('[data-queue]').innerText()), 'a prompt sent during the turn shows "1 queued"');
  await page.click('[data-queue] >> text=Drop the queue');
  await page.waitForSelector('[role=alertdialog]');
  check(/also stops the agent/.test(await page.locator('[role=alertdialog]').innerText()), 'dropping asks first and says it also stops the turn');
  await page.click('button:has-text("Drop and stop the turn")');
  check(await gone(page.locator('[data-queue]')), 'the queue goes');
  check(await gone(stopTurn), 'and the turn ended');
  check((await page.getByText('Not sent', { exact: true }).count()) === 1, 'the dropped prompt reads "Not sent"');

  // Stop turn alone ends the turn and leaves the agent alive.
  await say('slow');
  await stopTurn.waitFor();
  await stopTurn.click();
  check(await gone(stopTurn), 'Stop turn ends the turn');
  check(await page.locator('form textarea').isEnabled(), 'the agent stays and takes another message');

  // A request for permission is a record in the conversation and follows the user's answer in the pinned bar.
  await say('ask');
  await page.waitForSelector('[data-permission=pending]');
  check((await page.locator('[data-permission] button').count()) === 1, 'the record has only a way to the request, no Allow or Deny');
  // The card that floats over the window for a request that needs reading: put it aside.
  const later = page.getByRole('button', { name: 'Later', exact: true });
  if (await later.count()) await later.first().click();
  await page.click('[data-pinned-approvals] >> text=Allow');
  await page.waitForSelector('[data-permission=approved]');
  check(true, 'the record reads approved once the user allowed it');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  console.log('FAIL', e.stack ?? e.message);
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
