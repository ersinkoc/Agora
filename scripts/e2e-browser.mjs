// End-to-end check in a real browser: a hub and a node on this machine, sign in with a code, start a shell agent and type into
// its terminal, start a chat agent, send it a request, and answer its permission request from the window.
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-browser.mjs
//
// Needs git, sh and a Chromium (CHROME=<path>, default: Playwright's cache). Uses a throw-away folder; changes nothing else.
import { startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
let stack;

try {
  stack = await startStack();
  const { page, start, errors, origin } = stack;
  check(true, 'signed in with a code');

  await start('Shell');
  await page.waitForSelector('.xterm-helper-textarea', { state: 'attached', timeout: 10000 });
  // Type at the prompt, as a person does: Git for Windows' sh drops what arrives while it is still starting.
  await page.waitForFunction(() => /\$\s*$/.test((document.querySelector('.xterm-rows')?.innerText ?? '').trim()), null, { timeout: 30000 });
  await page.focus('.xterm-helper-textarea');
  await page.keyboard.type('echo hello-from-e2e\n', { delay: 5 });
  await page.waitForFunction(() => (document.querySelector('.xterm-rows')?.innerText ?? '').split('hello-from-e2e').length > 2, null, { timeout: 20000 }).catch(() => {});
  const screen = await page.innerText('.xterm-rows');
  check(screen.split('hello-from-e2e').length > 2, `typed text reaches the shell in order and its output comes back${screen.split('hello-from-e2e').length > 2 ? '' : `: ${screen.replace(/\s+/g, ' ').slice(0, 200)}`}`);

  // The harness trusted this browser on the node, so the chat agent's permission request can be answered from it.
  await start('Fake chat agent');
  const chat = page.url();
  await page.waitForSelector('form textarea', { timeout: 10000 });
  await page.fill('form textarea', 'ask');
  await page.click('button:has-text("Send")');
  // Requests are answered on Home (the agent's pane and the details panel); the full-size view shows only that it waits.
  await page.goto(`${origin}/#/`);
  const allow = page.getByRole('button', { name: 'Allow', exact: true }).locator('visible=true');
  await allow.first().waitFor({ timeout: 15000 });
  check(true, 'a request sent from the window makes the chat agent ask permission, and the request shows up');
  await allow.first().click();
  await page.waitForFunction(() => ![...document.querySelectorAll('button')].some(b => b.textContent === 'Allow' && b.offsetParent), null, { timeout: 10000 }).catch(() => {});
  check((await allow.count()) === 0, 'allowing it (signed by this browser) clears the request');
  await page.goto(chat);
  await page.waitForSelector('form textarea', { timeout: 10000 });
  // The conversation streams back: the request as a bubble, and the agent's own line (the fake agent echoes what it is told).
  await page.fill('form textarea', 'hello there');
  await page.click('button:has-text("Send")');
  // The message list (live region), not the pane around it, which carries the same label.
  await page.waitForFunction(() => (document.querySelector('[aria-live][aria-label="Conversation"]')?.innerText.match(/hello there/g) ?? []).length >= 2, null, { timeout: 8000 }).catch(() => {});
  const said = await page.locator('[aria-live][aria-label="Conversation"]').innerText();
  check((said.match(/hello there/g) ?? []).length >= 2, 'the chat agent\'s conversation shows the request and its answer');
  check((await page.locator('[role=alert]').count()) === 0, 'no error is shown');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  console.log('FAIL', e.message);
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
