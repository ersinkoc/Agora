// End-to-end check of the phone-sized window (ADR-0067 rung 2) in a real browser: a shell agent and a chat agent asking permission,
// then at 360 × 740, 390 × 844, 844 × 390 (a phone held sideways) and 768 × 1024 (a tablet) on a touch screen: the list shows what
// needs you first with Allow in reach, every control is at least 44 px, an agent opens in one column with Back, the key bar sends
// Esc, an arrow and Ctrl+C to the shell, and Allow answers the request (signed by this browser, as anywhere else). With the High
// contrast and Large text themes the 44 px targets and the key bar still hold, and the theme editor opens as a screen.
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-phone.mjs
//   AGORA_SCREENSHOTS=<dir> node scripts/e2e-phone.mjs   # also writes phone-*.png in both themes
//
// Needs git, sh and a Chromium (CHROME=<path>, default: Playwright's cache). Uses a throw-away folder; changes nothing else.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
const shots = process.env.AGORA_SCREENSHOTS;
if (shots) mkdirSync(shots, { recursive: true });
let stack;

// Whether the phone's screen scrolls sideways (something wider than the phone pushes the rest off it).
const sideways = (page) => page.evaluate(() => { const m = document.querySelector('.phone__main'); return !!m && m.scrollWidth > m.clientWidth + 1; });

// Controls a finger presses in the phone window (and the key bar) that are smaller than 44 px.
const tooSmall = (page) => page.evaluate(() => [...document.querySelectorAll('.phone button, .phone a[href], .key-bar button')]
  .filter((e) => e.offsetParent && !e.closest('[role=dialog]'))
  .map((e) => [e, e.getBoundingClientRect()])
  .filter(([, r]) => r.width > 0 && (r.width < 43.5 || r.height < 43.5))
  .map(([e, r]) => `${e.getAttribute('aria-label') || e.textContent.trim().slice(0, 20)} ${Math.round(r.width)}×${Math.round(r.height)}`));

const screen = (page) => page.evaluate(() => document.querySelector('.xterm-rows')?.innerText ?? '');

try {
  stack = await startStack({ viewport: { width: 1440, height: 900 } });
  const { page, start, errors, origin } = stack;
  await start('Shell');
  const shell = decodeURIComponent(page.url().split('#/agent/')[1]);
  await page.waitForFunction(() => /\$\s*$/.test((document.querySelector('.xterm-rows')?.innerText ?? '').trim()), null, { timeout: 30000 });
  await start('Fake chat agent');
  await page.waitForSelector('form textarea', { timeout: 10000 });
  await page.fill('form textarea', 'ask');
  await page.click('button:has-text("Send")');

  // A desktop window is the window it was: no phone column, no key bar.
  await page.goto(`${origin}/#/agent/${encodeURIComponent(shell)}`);
  await page.waitForSelector('.xterm-rows', { timeout: 10000 });
  check(!(await page.locator('.phone, .key-bar').count()), 'a desktop window has neither the phone column nor the key bar');

  // A touch screen, as a phone's browser reports it.
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
  check(await page.evaluate(() => matchMedia('(pointer: coarse)').matches), 'the browser reports a touch screen');

  for (const [w, h] of [[360, 740], [390, 844], [844, 390], [768, 1024]]) {
    await page.setViewportSize({ width: w, height: h });
    const tag = `${w}x${h}`;
    await page.goto(`${origin}/#/`);
    if (w === 768) {
      // A tablet keeps the full window; its terminals get the key bar.
      await page.waitForSelector('.ide', { timeout: 10000 });
      check(!(await page.locator('.phone').count()), `${tag}: a tablet keeps the full window`);
      await page.goto(`${origin}/#/agent/${encodeURIComponent(shell)}`);
      await page.waitForSelector('.key-bar', { timeout: 10000 });
      check(true, `${tag}: the terminal has the key bar`);
      for (const scheme of ['light', 'dark']) {
        await page.emulateMedia({ colorScheme: scheme });
        await page.waitForTimeout(400);
        if (shots) await page.screenshot({ path: join(shots, `phone-${tag}-agent-${scheme}.png`) });
      }
      continue;
    }
    await page.waitForSelector('.phone', { timeout: 10000 });
    const allow = page.locator('.phone').getByRole('button', { name: 'Allow', exact: true });
    await allow.first().waitFor({ timeout: 15000 });
    const box = await allow.first().boundingBox();
    check(!!box && box.y + box.height <= h, `${tag}: Allow is on the first screen, before any scrolling`);
    const first = await page.locator('.phone-section__title').first().innerText();
    check(/needs you/i.test(first), `${tag}: "Needs you" comes first (${first.replace(/\s+/g, ' ')})`);
    const small = await tooSmall(page);
    check(small.length === 0, `${tag}: every control on the list is at least 44 px${small.length ? `: ${small.join(', ')}` : ''}`);
    check(!(await sideways(page)), `${tag}: nothing on the list is wider than the screen`);
    for (const scheme of ['light', 'dark']) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.waitForTimeout(400); // colours change with a short transition
      if (shots) await page.screenshot({ path: join(shots, `phone-${tag}-home-${scheme}.png`) });
    }

    // One tap on the agent opens it, in one column, with Back.
    await page.locator('.phone-row', { hasText: 'shell' }).first().click();
    await page.waitForSelector('.key-bar', { timeout: 10000 });
    await page.waitForFunction(() => /\$\s*$/.test((document.querySelector('.xterm-rows')?.innerText ?? '').trim()), null, { timeout: 30000 });
    const small2 = await tooSmall(page);
    check(small2.length === 0, `${tag}: every control on the agent is at least 44 px${small2.length ? `: ${small2.join(', ')}` : ''}`);
    const fits = await page.evaluate(() => { const b = document.querySelector('.key-bar'); return !!b && b.scrollWidth <= b.clientWidth; });
    check(fits, `${tag}: the whole key bar fits the width`);
    const term = await page.locator('.phone [role=region].terminal').boundingBox();
    check(!!term && term.width >= w - 40, `${tag}: the terminal takes the width (${Math.round(term?.width ?? 0)} px)`);
    for (const scheme of ['light', 'dark']) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.waitForTimeout(400); // colours change with a short transition
      if (shots) await page.screenshot({ path: join(shots, `phone-${tag}-agent-${scheme}.png`) });
    }
    await page.getByRole('button', { name: 'Back to the list', exact: true }).click();
    await page.waitForFunction(() => location.hash === '#/', null, { timeout: 5000 });
    check(true, `${tag}: Back returns to the list`);
  }

  // The window's other pages keep their frame on a phone, with a Back a finger can press.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${origin}/#/device`);
  await page.waitForSelector('.codebox, header.ide-bar', { timeout: 10000 });
  const back = await page.getByRole('button', { name: 'Back to Home', exact: true }).boundingBox();
  check(!!back && back.height >= 43.5, `a page's Back is at least 44 px tall (${Math.round(back?.height ?? 0)})`);
  for (const scheme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(400);
    if (shots) await page.screenshot({ path: join(shots, `phone-390x844-device-${scheme}.png`) });
  }

  // ADR-0068 themes on a phone: High contrast and Large text keep 44 px targets and a key bar that fits; the theme editor is a
  // screen of its own with Back.
  const pickTheme = async (name) => {
    await page.goto(`${origin}/#/`);
    await page.locator('.phone').getByRole('button', { name: 'Search', exact: true }).click();
    await page.keyboard.type(name);
    await page.keyboard.press('Enter');
    await page.waitForTimeout(400);
  };
  await page.emulateMedia({ colorScheme: 'dark' });
  for (const [theme, file] of [['Theme: High contrast', 'high-contrast'], ['Theme: Large text', 'large-text']]) {
    for (const [w, h] of [[360, 740], [390, 844]]) {
      const tag = `${w}x${h} ${file}`;
      await page.setViewportSize({ width: w, height: h });
      await pickTheme(theme);
      await page.waitForSelector('.phone', { timeout: 10000 });
      const small = await tooSmall(page);
      check(small.length === 0, `${tag}: every control on the list is at least 44 px${small.length ? `: ${small.join(', ')}` : ''}`);
      check(!(await sideways(page)), `${tag}: nothing on the list is wider than the screen`);
      if (shots) await page.screenshot({ path: join(shots, `phone-${w}x${h}-home-${file}.png`) });
      await page.goto(`${origin}/#/agent/${encodeURIComponent(shell)}`);
      await page.waitForSelector('.key-bar', { timeout: 10000 });
      await page.waitForTimeout(300);
      const small2 = await tooSmall(page);
      check(small2.length === 0, `${tag}: every control on the agent is at least 44 px${small2.length ? `: ${small2.join(', ')}` : ''}`);
      const fits = await page.evaluate(() => { const b = document.querySelector('.key-bar'); return !!b && b.scrollWidth <= b.clientWidth; });
      check(fits, `${tag}: the whole key bar fits the width`);
      if (shots) await page.screenshot({ path: join(shots, `phone-${w}x${h}-agent-${file}.png`) });
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await pickTheme('Edit theme');
  await page.waitForFunction(() => location.hash === '#/theme', null, { timeout: 5000 }).catch(() => {});
  check(await page.locator('.phone section[aria-label="Theme"]').count() === 1, 'on a phone, Edit theme opens the editor as a screen of its own');
  if (shots) await page.screenshot({ path: join(shots, 'phone-390x844-theme-editor-large-text.png') });
  await page.getByRole('button', { name: 'Back to the list', exact: true }).click();
  await page.waitForFunction(() => location.hash === '#/', null, { timeout: 5000 });
  check(true, 'Back leaves the theme editor for the list');
  await pickTheme('Theme: Dark');

  // The key bar's bytes reach the program: `cat -v` shows Esc and an arrow as text; Ctrl then c ends it.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${origin}/#/agent/${encodeURIComponent(shell)}`);
  await page.waitForSelector('.key-bar', { timeout: 10000 });
  await page.focus('.xterm-helper-textarea');
  await page.keyboard.type('cat -v\n', { delay: 5 });
  await page.waitForTimeout(500);
  const bar = page.getByRole('toolbar', { name: 'Terminal keys' });
  await bar.getByRole('button', { name: 'Esc', exact: true }).click();
  await bar.getByRole('button', { name: 'Left', exact: true }).click();
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => (document.querySelector('.xterm-rows')?.innerText ?? '').includes('^[^[[D'), null, { timeout: 5000 }).catch(() => {});
  check((await screen(page)).includes('^[^[[D'), 'Esc and Left from the key bar reach the program as ESC and ESC [ D');
  const focused = await page.evaluate(() => document.activeElement?.classList.contains('xterm-helper-textarea'));
  check(focused, 'pressing a key bar key keeps the focus in the terminal (the touch keyboard stays open)');
  await bar.getByRole('button', { name: 'Ctrl', exact: true }).click();
  check((await bar.getByRole('button', { name: 'Ctrl', exact: true }).getAttribute('aria-pressed')) === 'true', 'Ctrl shows that it is on');
  await page.keyboard.type('c');
  await page.waitForTimeout(300);
  check((await bar.getByRole('button', { name: 'Ctrl', exact: true }).getAttribute('aria-pressed')) === 'false', 'Ctrl lets go after one key');
  await page.keyboard.type('echo after-ctrl-c\n', { delay: 5 });
  await page.waitForFunction(() => (document.querySelector('.xterm-rows')?.innerText ?? '').split('after-ctrl-c').length > 2, null, { timeout: 5000 }).catch(() => {});
  check((await screen(page)).split('after-ctrl-c').length > 2, 'Ctrl then c ends `cat` (the shell runs the next command)');

  // Allow on the list answers the request, signed by this browser like anywhere else.
  await page.goto(`${origin}/#/`);
  const allow = page.locator('.phone').getByRole('button', { name: 'Allow', exact: true });
  await allow.first().click();
  await page.waitForFunction(() => !document.querySelector('.phone-card--needs .button--primary'), null, { timeout: 10000 }).catch(() => {});
  check((await allow.count()) === 0, 'one tap on Allow answers the request and it leaves the list');
  check((await page.locator('[role=alert]').count()) === 0, 'no error is shown');

  // A notice's link to an approval opens the agent that asks (here: answered already, so the list).
  await page.goto(`${origin}/#/approvals/laptop%2Fnone`);
  await page.waitForFunction(() => location.hash === '#/', null, { timeout: 5000 }).catch(() => {});
  check(await page.evaluate(() => location.hash === '#/'), 'a link to an approval that is already answered opens the list');

  // The install manifest names icons the hub serves.
  const manifest = await (await page.request.get(`${origin}/manifest.webmanifest`)).json();
  for (const icon of manifest.icons) {
    const r = await page.request.get(`${origin}${icon.src}`);
    check(r.ok() && (r.headers()['content-type'] ?? '').startsWith(icon.type), `the manifest's ${icon.src} is served as ${icon.type}`);
  }
  check(manifest.icons.some((i) => i.purpose === 'maskable'), 'the manifest has a maskable icon');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  console.log('FAIL', e.message);
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
