// Settings → Agents against a real hub and node: the section is linked and routed, lists installed and not installed agents with
// what each takes, labels an option that skips permissions, shows third-party text as plain text, and offers no install action.
// AGORA_SCREENSHOTS=<dir> also saves screenshots in both themes and at phone width.
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

let stack;
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page, home, origin, errors } = stack;
  page.setDefaultTimeout(12000);
  const manifest = (kind, name, program, extra = '') => writeFileSync(join(home, 'manifests', `${kind}.toml`), `kind = "${kind}"\ndisplay_name = ${JSON.stringify(name)}\nversion = "1"\nmin_engine = 3\n[process]\nlaunch = ["${program}"]\n${extra}`);
  manifest('present', 'Present Agent', 'sh', '[[launch.option]]\nid = "skip-permissions"\nlabel = "option-skip-permissions"\nargs = ["--x"]\nskips_permissions = true\n[[launch.setting]]\nid = "effort"\nlabel = "setting-effort"\nargs = ["--effort", "{value}"]\nvalues = ["low", "high"]\ndefault = "low"\n');
  manifest('absent', '<img src=x onerror=alert(1)> **bold** [x](https://evil.example)', 'no-such-program-here');
  await page.goto(`${origin}/#/device/agents`);
  await page.reload();
  const section = page.locator('#settings-agents');
  await section.getByText('Present Agent').waitFor();
  const text = await section.innerText();
  assert.match(text, /installed/i);
  assert.match(text, /not installed/i);
  // A kind that offers to act without asking carries the red bolt, its meaning in its name; its card, opened, labels the option.
  assert.ok(await section.getByRole('img', { name: /skips permission questions/ }).count() > 0);
  await section.locator('summary', { hasText: 'Present Agent' }).click();
  assert.match(await section.innerText(), /skips permission questions/);
  assert.equal(await section.locator('img, a[href^="https://evil"]').count(), 0, 'untrusted text must stay plain');
  assert.match(text, /<img src=x/);
  // The only buttons are the machine choice (radios): nothing installs, starts or saves.
  assert.equal(await section.locator('button:not([role="radio"])').count(), 0, 'no install or save action');
  assert.ok(await page.locator('nav a[href="#/device/agents"]').count() > 0);
  const wide = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
  assert.equal(wide, false, 'no horizontal page scroll');
  const shots = process.env.AGORA_SCREENSHOTS;
  if (shots) {
    mkdirSync(shots, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme: theme });
      await section.scrollIntoViewIfNeeded();
      await page.screenshot({ path: join(shots, `settings-agents-${theme}.png`), animations: 'disabled' });
    }
    await page.setViewportSize({ width: 390, height: 800 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'phone width');
    await page.screenshot({ path: join(shots, 'settings-agents-phone.png'), animations: 'disabled' });
  }
  assert.deepEqual(errors, []);
  console.log('settings agents ok');
} finally {
  await stack?.close();
}
