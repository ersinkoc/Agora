// #284 / LAN L19: change language in a real browser without losing a terminal, form or connection.
// Uses a disposable hub/node and a shell, never a real coding agent or account.
import assert from 'node:assert/strict';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { openStart, startStack } from './browser-harness.mjs';

let stack;
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page, start, errors, origin } = stack;
  page.setDefaultTimeout(10000);
  const choice = () => page.getByRole('combobox', { name: /^(Language|Dil)$/ });
  await choice().waitFor({ timeout: 2000 });
  await start('Shell');
  // Starting opens the agent full size; the checks below run on Home, where its pane is open.
  await page.goto(`${origin}/#/`);
  const xterm = page.locator('.xterm').first();
  await xterm.waitFor();
  await xterm.evaluate(el => { el.dataset.languageProbe = 'original-terminal'; });
  let reconnects = 0;
  page.on('websocket', () => { reconnects++; });

  // The start window is modal (#332), so the page's own language choice is out of reach while it is open: the language is
  // changed in a second tab, which every open tab follows.
  const other = await page.context().newPage();
  await other.goto(origin);
  const fromOtherTab = async lang => {
    await other.getByRole('combobox', { name: /^(Language|Dil)$/ }).selectOption(lang);
    await page.waitForFunction(want => document.documentElement.lang === want, lang);
  };
  await openStart(page);
  const folder = page.locator('[role=dialog] input:not([type=checkbox])');
  await folder.fill('/some/chosen/folder');
  await fromOtherTab('tr');
  await page.getByRole('button', { name: 'Bu klasörü kullan', exact: true }).waitFor();
  assert.equal(await folder.inputValue(), '/some/chosen/folder');
  await page.getByRole('button', { name: 'Bu klasörü kullan', exact: true }).click();
  const folderError = page.locator('[role=dialog] [role=alert]');
  await folderError.waitFor();
  const turkishError = await folderError.innerText();
  await fromOtherTab('en');
  await page.waitForFunction(previous => document.querySelector('[role=dialog] [role=alert]')?.textContent !== previous, turkishError);
  await fromOtherTab('tr');
  await other.close();
  assert.equal(await folderError.innerText(), turkishError);
  assert.equal(await xterm.getAttribute('data-language-probe'), 'original-terminal');
  assert.equal(await page.locator('html').getAttribute('lang'), 'tr');
  assert.equal(await page.locator('html').getAttribute('dir'), 'ltr');
  assert.equal(reconnects, 0, 'Language choice must not reconnect a terminal or the hub');
  // The text button, not the window's close icon, which has the same name.
  await page.getByRole('dialog').getByText('Vazgeç', { exact: true }).click();
  await page.keyboard.press('Control+k');
  await page.getByRole('option', { name: 'Ana sayfaya git', exact: true }).waitFor();
  await page.keyboard.press('Escape');

  const screenshots = process.env.AGORA_SCREENSHOTS;
  const shoot = async name => {
    if (!screenshots) return;
    mkdirSync(screenshots, { recursive: true });
    for (const theme of ['dark', 'light']) {
      // Mount xterm with this theme too; changing only the page's tokens leaves its existing canvas unchanged.
      await page.emulateMedia({ colorScheme: theme });
      await page.reload();
      await page.locator('.xterm').first().waitFor();
      await page.screenshot({ path: join(screenshots, `${name}-${theme}.png`), fullPage: true, animations: 'disabled' });
    }
  };
  await shoot('home-tr-desktop');
  await page.setViewportSize({ width: 390, height: 844 });
  await shoot('home-tr-phone');
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'Language selector must fit on a phone');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.reload();
  await choice().waitFor();
  assert.equal(await choice().inputValue(), 'tr');
  assert.equal(await page.locator('html').getAttribute('lang'), 'tr');

  // Focus labels change without disposing the existing renderer either.
  await page.getByRole('button', { name: 'Tam boyut', exact: true }).first().click();
  const focusTerminal = page.locator('.xterm');
  await focusTerminal.waitFor();
  await focusTerminal.evaluate(el => { el.dataset.languageProbe = 'focused-terminal'; });
  await choice().selectOption('en');
  await page.getByRole('button', { name: 'Back to Home', exact: true }).waitFor();
  assert.equal(await focusTerminal.getAttribute('data-language-probe'), 'focused-terminal');
  await page.getByRole('button', { name: 'Back to Home', exact: true }).click();
  // Home is drawn: until then the first h1 is still the agent's name, which the timing below would wait on for ever (macOS).
  await page.waitForFunction(() => document.querySelector('h1')?.textContent === 'Home');
  await shoot('home-en-desktop');
  await choice().selectOption('system');
  assert.equal(await page.evaluate(() => localStorage.getItem('ui.language')), null);
  assert.equal(await page.locator('html').getAttribute('lang'), 'en');

  const samples = await page.evaluate(async () => {
    const choice = document.querySelector('header select');
    const times = [];
    for (let i = 0; i < 20; i++) {
      const lang = i % 2 ? 'en' : 'tr';
      const started = performance.now();
      await new Promise((resolve, reject) => {
        // A sample that never arrives fails with what was shown, instead of hanging the script.
        const late = setTimeout(() => { observer.disconnect(); reject(new Error(`no ${lang} heading, h1: ${document.querySelector('h1')?.textContent}`)); }, 5000);
        const observer = new MutationObserver(() => {
          if (document.querySelector('h1')?.textContent === (lang === 'tr' ? 'Ana sayfa' : 'Home')) {
            clearTimeout(late); observer.disconnect(); resolve();
          }
        });
        observer.observe(document.getElementById('root'), { childList: true, subtree: true, characterData: true });
        choice.value = lang;
        choice.dispatchEvent(new Event('change', { bubbles: true }));
      });
      times.push(performance.now() - started);
    }
    return times.sort((a, b) => a - b);
  });
  console.log(`Language render, one live shell, 20 samples: p95 ${samples[19].toFixed(1)} ms (local render target 50 ms).`);

  // Choices made in another tab are reflected without a page reload.
  const otherTab = await page.context().newPage();
  await otherTab.goto(origin);
  await otherTab.getByRole('combobox', { name: 'Language', exact: true }).selectOption('tr');
  await page.waitForFunction(() => document.documentElement.lang === 'tr');
  await otherTab.close();

  // Browser storage can be blocked: still change the current page and explain that it will not be saved.
  await page.evaluate(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function(key, value) {
      if (key === 'ui.language') throw new DOMException('Storage refused', 'SecurityError');
      return original.call(this, key, value);
    };
  });
  await choice().selectOption('en');
  await page.getByRole('status').filter({ hasText: 'Your browser could not save the preference.' }).waitFor();
  assert.equal(await page.locator('html').getAttribute('lang'), 'en');

  // A fresh, signed-out tab can choose its language before typing a code. It persists through reload.
  const signedOut = await page.context().browser().newContext({ ignoreHTTPSErrors: true, locale: 'en-US' });
  const login = await signedOut.newPage();
  login.on('pageerror', error => errors.push(error.message));
  await login.goto(origin);
  const loginChoice = () => login.getByRole('combobox', { name: /^(Language|Dil)$/ });
  await login.getByRole('textbox').fill('UNSUBMITTED');
  await loginChoice().selectOption('tr');
  assert.equal(await login.getByRole('textbox').inputValue(), 'UNSUBMITTED');
  await login.reload();
  assert.equal(await loginChoice().inputValue(), 'tr');
  assert.equal(await login.locator('html').getAttribute('lang'), 'tr');
  await login.evaluate(() => { localStorage.setItem('ui.language', 'unsupported-old-preference'); });
  await login.reload();
  assert.equal(await loginChoice().inputValue(), 'system');
  assert.equal(await login.locator('html').getAttribute('lang'), 'en');
  await loginChoice().selectOption('tr');
  for (const [name, viewport] of [['desktop', { width: 1280, height: 900 }], ['phone', { width: 390, height: 844 }]]) {
    await login.setViewportSize(viewport);
    for (const theme of ['dark', 'light']) {
      await login.evaluate(value => { document.documentElement.dataset.theme = value; }, theme);
      if (screenshots) await login.screenshot({ path: join(screenshots, `signin-tr-${name}-${theme}.png`), fullPage: true, animations: 'disabled' });
    }
  }
  await login.getByRole('textbox').fill('AAAA-BBBB');
  await login.locator('button[type=submit]').click();
  await login.getByRole('alert').waitFor();
  const loginError = await login.getByRole('alert').innerText();
  await loginChoice().selectOption('en');
  await login.waitForFunction(previous => document.querySelector('[role=alert]')?.textContent !== previous, loginError);
  await signedOut.close();
  if (errors.length) throw new Error(errors.join('\n'));
  console.log('PASS: language changes preserve terminal renderer, connection and working-folder form; palette updates, saved choice survives reload, browser default returns, and sign-in choice is available before authentication.');
} catch (error) {
  console.error(error.stack || error);
  if (stack) console.error((await stack.page.locator('body').innerText()).slice(0, 2000));
  process.exitCode = 1;
} finally { await stack?.close(); }
