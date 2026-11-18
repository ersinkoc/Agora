// Screenshots of the "Needs you" card, banner and This device settings (docs/ux/main-window.md, "Needs you, loudly"), in both themes, from the
// web app's demo window (no hub). The demo has one plain request; to see the card for a request that is only reviewed, run with REVIEW=1
// against a window whose demo request is a start request. Needs the dev server or a built app, and playwright-core with a Chromium:
//   cd apps/web && npx vite --port 5199 &
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/screenshots-needs-you-modal.mjs http://127.0.0.1:5199/ [outdir]
import { createRequire } from 'node:module';
import { join } from 'node:path';
const base = process.argv[2] ?? 'http://127.0.0.1:5199/';
const out = process.argv[3] ?? new URL('../docs/screenshots/needs-you-modal', import.meta.url).pathname;
const review = process.env.REVIEW === '1';
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
for (const scheme of ['dark', 'light']) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
  const page = await ctx.newPage();
  await page.goto(`${base}#/demo`);
  await page.locator('.attention-card').waitFor({ timeout: 10000 });
  await page.waitForTimeout(900); // past the dwell, so the buttons look ready
  await page.screenshot({ path: join(out, `${review ? 'card-review' : 'card-answer'}-${scheme}.png`) });
  if (!review) {
    await page.keyboard.press('Escape'); // Esc is Later
    await page.locator('.attention-banner').waitFor();
    await page.screenshot({ path: join(out, `banner-${scheme}.png`) });
    await page.evaluate(() => { window.location.hash = '#/device'; });
    await page.getByRole('heading', { name: 'Needs you', exact: true }).waitFor();
    await page.screenshot({ path: join(out, `this-device-${scheme}.png`) });
  }
  await ctx.close();
}
await browser.close();
