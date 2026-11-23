// Settings → Screen password in the hub machine's own browser, when the password was set while the window was open (#909): the
// window is signed out, signs in again with a code, and Settings must then know a password is set: it offers "Change password" with
// a "Current password" field, refuses a wrong current password, and changes it with the right one. The password is set from a second
// window (like `agora password set`, it signs out every other window). AGORA_SCREENSHOTS=<dir> also saves the section in both themes.
import assert from 'node:assert/strict';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

const FIRST = 'first-screen-pass';
const SECOND = 'second-screen-pass';
let stack;
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page, run, hubDir, origin, errors } = stack;
  page.setDefaultTimeout(15000);
  const code = () => run(['hub', 'login', '--hub-dir', hubDir]).match(/app: (\S+) \(/)[1];
  // The window was open before any password: Settings offers to set one.
  await page.goto(`${origin}/#/device/lock`);
  const section = page.locator('#settings-lock');
  await section.getByRole('button', { name: 'Set password', exact: true }).waitFor();

  // A second window on this machine sets the password; every other session ends.
  const other = await (await page.context().browser().newContext({ ignoreHTTPSErrors: true })).newPage();
  await other.goto(`${origin}/`);
  await other.fill('input', code());
  await other.click('button[type=submit]');
  await other.waitForSelector('text=Start an agent');
  const status = await other.evaluate(async (password) => {
    const token = localStorage.getItem('agora.hubToken');
    const r = await fetch('/api/password', { method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: JSON.stringify({ password }) });
    return r.status;
  }, FIRST);
  assert.equal(status, 200, 'the password is set');
  await other.context().close();

  // The first window is signed out and signs in again with a new code, without reloading the page.
  await page.getByRole('heading', { name: 'Sign in to your Agora', exact: true }).waitFor({ timeout: 30000 });
  await page.fill('input', code());
  await page.click('button[type=submit]');
  await page.waitForSelector('text=Start an agent');
  await page.evaluate(() => { window.location.hash = '#/device/lock'; });
  const change = section.getByRole('button', { name: 'Change password', exact: true });
  await change.waitFor();
  const current = section.locator('input[name=current]');
  assert.equal(await current.count(), 1, 'Settings asks for the current password');

  const fill = async (now) => {
    await current.fill(now);
    await section.locator('input[name=new]').fill(SECOND);
    await section.locator('input[name=again]').fill(SECOND);
    await change.click();
  };
  await fill('not-the-password');
  await section.getByText('That password is not right.').waitFor();
  await fill(FIRST);
  await section.getByText('Saved. Every other window was signed out.').waitFor();

  const shots = process.env.AGORA_SCREENSHOTS;
  if (shots) {
    mkdirSync(shots, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme: theme });
      await section.scrollIntoViewIfNeeded();
      await page.screenshot({ path: join(shots, `settings-password-changed-${theme}.png`), animations: 'disabled' });
    }
  }
  assert.deepEqual(errors, []);
  console.log('screen password ok');
} finally {
  await stack?.close();
}
