// ADR-0113 step 4 in a real browser on a real hub: the same person signed in in two browsers (the laptop and the phone, each with its own
// storage) sees the same themes and theme choice after a change in either. The hub keeps them as the person's own opaque value; every
// request the pages make stays on the hub's origin. Uses a disposable hub and node, never a real account.
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-theme-sync.mjs
import assert from 'node:assert/strict';
import { startStack } from './browser-harness.mjs';

const MEADOW = { 'agora-theme': 2, name: 'Meadow', inherits: 'sand', light: { tokens: { accent: '#2f6b3a', 'on-accent': '#ffffff' } } };
const PACK = JSON.stringify({ 'agora-pack': 1, name: 'Meadow set', themes: [MEADOW] });
const HARBOUR = JSON.stringify({ 'agora-theme': 2, name: 'Harbour', inherits: 'slate', dark: { tokens: { accent: '#5cc8c2' } }, light: { tokens: { accent: '#0b6e69', 'on-accent': '#ffffff' } } });

let stack;
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page: laptop, run, hubDir, origin, errors } = stack;
  laptop.setDefaultTimeout(15000);
  const elsewhere = [];
  const watch = (page) => page.on('request', (r) => { const u = new URL(r.url()); if (u.origin !== new URL(origin).origin && !['data:', 'blob:'].includes(u.protocol)) elsewhere.push(r.url()); });
  watch(laptop);
  const openEditor = async (page) => {
    await page.goto(`${origin}/#/`);
    await page.waitForSelector('.ide');
    await page.keyboard.press('Control+k');
    await page.keyboard.type('Edit theme');
    await page.keyboard.press('Enter');
    await page.waitForSelector('section[aria-label="Theme"]');
  };
  const choice = (page) => page.evaluate(() => JSON.parse(localStorage.getItem('agora.theme.choice.v1') ?? 'null'));
  const names = (page) => page.evaluate(() => JSON.parse(localStorage.getItem('agora.themes.v1') ?? '[]').map((e) => e.file.name));

  // The laptop imports a theme (it becomes the dark choice) and writes it to the hub after the change settles.
  await openEditor(laptop);
  await laptop.locator('section[aria-label="Theme"] input[type=file]:not([data-import-other])').setInputFiles({ name: 'harbour.json', mimeType: 'application/json', buffer: Buffer.from(HARBOUR) });
  await laptop.waitForFunction(() => JSON.parse(localStorage.getItem('agora.themes.v1') ?? '[]').some((e) => e.file?.name === 'Harbour'));
  const laptopChoice = await choice(laptop);
  assert.match(laptopChoice.dark, /^user-\d+$/);
  // And imports a pack, adding its theme: the pack is kept for the person's other devices.
  await laptop.getByRole('button', { name: 'Import pack…' }).click();
  await laptop.getByLabel('Paste a pack (JSON)').fill(PACK);
  await laptop.getByRole('button', { name: "See what's in it" }).click();
  await laptop.locator('[data-pack-dialog="preview"] input[type=checkbox]').first().check();
  await laptop.getByRole('button', { name: "Use what's ticked" }).click();
  await laptop.getByRole('button', { name: 'Close' }).first().click();
  await laptop.locator('[data-kept-pack]').waitFor();
  await laptop.waitForTimeout(3000); // the write waits 1.5 s after the last change

  // The phone: another browser, its own storage, the same person. It reads on sign-in and shows the laptop's theme as its choice.
  const ctx = await laptop.context().browser().newContext({ ignoreHTTPSErrors: true, viewport: { width: 390, height: 844 } });
  const phone = await ctx.newPage();
  watch(phone);
  phone.on('pageerror', (e) => errors.push(e.message));
  phone.setDefaultTimeout(15000);
  await phone.goto(`${origin}/`);
  await phone.fill('input', run(['hub', 'login', '--hub-dir', hubDir]).match(/app: (\S+) \(/)[1]);
  await phone.click('button[type=submit]');
  await phone.waitForFunction(() => JSON.parse(localStorage.getItem('agora.themes.v1') ?? '[]').some((e) => e.file?.name === 'Harbour'), null, { timeout: 20000 });
  assert.deepEqual((await names(phone)).sort(), ['Harbour', 'Meadow']);
  const kept = await phone.evaluate(() => JSON.parse(localStorage.getItem('agora.packs.v1') ?? '[]').map((p) => p.file.name));
  assert.deepEqual(kept, ['Meadow set'], 'the pack came too, kept, not applied beyond what the laptop added');
  const phoneChoice = await choice(phone);
  assert.match(phoneChoice.dark, /^user-\d+$/, 'the phone chose the synced theme');
  const accent = (page) => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--accent').trim());
  // Both pages draw the same accent (a scheme switch repaints a moment later, so this waits for it).
  const sameAccent = async (what) => {
    for (let i = 0; i < 50; i++) { if (await accent(phone) === await accent(laptop)) return; await laptop.waitForTimeout(100); }
    assert.equal(await accent(phone), await accent(laptop), what);
  };
  await phone.emulateMedia({ colorScheme: 'dark' });
  await laptop.emulateMedia({ colorScheme: 'dark' });
  await sameAccent('both show the same theme');

  // Settings > This device says it is on.
  await phone.goto(`${origin}/#/device/appearance`);
  const toggle = phone.getByRole('switch', { name: 'Keep my themes the same on my devices' });
  await toggle.waitFor();
  assert.equal(await toggle.getAttribute('aria-checked'), 'true');

  // The phone chooses Slate; the laptop has it when it comes back into view.
  await phone.goto(`${origin}/#/`);
  await phone.waitForSelector('.ide, .phone', { timeout: 15000 }).catch(() => {});
  await phone.keyboard.press('Control+k');
  await phone.keyboard.type('Theme: Slate');
  await phone.keyboard.press('Enter');
  await phone.waitForFunction(() => JSON.parse(localStorage.getItem('agora.theme.choice.v1') ?? 'null')?.dark === 'slate');
  await phone.waitForTimeout(3000);
  await laptop.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
  await laptop.waitForFunction(() => JSON.parse(localStorage.getItem('agora.theme.choice.v1') ?? 'null')?.dark === 'slate', null, { timeout: 15000 });
  await sameAccent('both show Slate');
  assert.deepEqual((await names(laptop)).sort(), ['Harbour', 'Meadow'], 'the themes stay');

  // Screenshots for docs/ux/themes.md (SHOTS=<folder>): the switch in Settings > This device > Appearance, on, in both schemes.
  if (process.env.SHOTS) {
    await laptop.goto(`${origin}/#/device/appearance`);
    for (const scheme of ['dark', 'light']) {
      await laptop.evaluate((s) => { localStorage.setItem('agora.theme', s); window.dispatchEvent(new StorageEvent('storage', { key: 'agora.theme' })); }, scheme);
      const card = laptop.locator('#settings-appearance');
      await card.waitFor();
      await laptop.waitForTimeout(300);
      await card.screenshot({ path: `${process.env.SHOTS}/adr113-theme-sync-setting-${scheme}.png` });
    }
    // The theme editor's "Your packs and devices", with the kept pack.
    for (const scheme of ['dark', 'light']) {
      await openEditor(laptop);
      await laptop.evaluate((s) => { localStorage.setItem('agora.theme', s); window.dispatchEvent(new StorageEvent('storage', { key: 'agora.theme' })); }, scheme);
      const section = laptop.locator('[data-theme-synced]');
      await section.waitFor();
      await section.scrollIntoViewIfNeeded();
      await laptop.waitForTimeout(300);
      await section.screenshot({ path: `${process.env.SHOTS}/adr113-theme-synced-packs-${scheme}.png` });
    }
  }

  // The write is in the owner's Activity, with no part of the value.
  const audit = run(['hub', 'audit', 'export', '--hub-dir', hubDir]);
  assert.match(audit, /settings\.set/, audit);
  assert.doesNotMatch(audit, /Harbour/, 'the audit trail holds nothing of the value');

  assert.deepEqual(elsewhere, [], 'the windows asked no other origin');
  assert.deepEqual(errors, []);
  console.log('ok: a theme imported on one browser and a choice made on another reached the other through the hub, same origin only');
} finally {
  await stack?.close();
}
