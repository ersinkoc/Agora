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
  const { page, run, start, errors } = stack;
  check(true, 'signed in with a code');

  await start('Shell');
  await page.waitForSelector('.xterm-helper-textarea', { state: 'attached', timeout: 10000 });
  await page.focus('.xterm-helper-textarea');
  await page.keyboard.type('echo hello-from-e2e\n', { delay: 5 });
  await page.waitForFunction(() => (document.querySelector('.xterm-rows')?.innerText ?? '').split('hello-from-e2e').length > 2, null, { timeout: 8000 }).catch(() => {});
  const screen = await page.innerText('.xterm-rows');
  check(screen.split('hello-from-e2e').length > 2, 'typed text reaches the shell in order and its output comes back');

  // Trust this browser on the node, then the chat agent's permission request can be answered.
  await page.goto('https://localhost:7443/#/device');
  const trust = (await page.innerText('.codebox')).trim().replace(/^agora /, '');
  run(trust.split(' '));
  await page.goto('https://localhost:7443/#/');
  await start('Fake chat agent');
  await page.waitForSelector('form textarea', { timeout: 10000 });
  await page.fill('form textarea', 'ask');
  await page.click('button:has-text("Send")');
  await page.waitForSelector('button:has-text("Allow")', { timeout: 10000 });
  check(true, 'a request sent from the window makes the chat agent ask permission, and the request shows up');
  await page.click('button:has-text("Allow")');
  await page.waitForTimeout(2500);
  check((await page.locator('button:has-text("Allow")').count()) === 0, 'allowing it (signed by this browser) clears the request');
  check((await page.locator('[role=alert]').count()) === 0, 'no error is shown');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  console.log('FAIL', e.message);
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
