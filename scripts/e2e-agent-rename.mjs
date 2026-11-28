// Real-browser check for naming an agent from the agent list (net/agentNames.ts, docs/ux/main-window.md), on the built demo window
// (#/demo), in dark and light:
//   1. The pencil is hidden until the row is hovered, then shown beside the ⋯ menu; the row stays one line.
//   2. The pencil turns the name into a field holding the name, selected; Enter keeps the new name in the row and in the agent's tab.
//   3. The name survives a reload (this browser keeps it); F2 on the row and an empty field give back the automatic name.
//
//   cd apps/web && npm ci && npm run gen && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-agent-rename.mjs
//   AGORA_SCREENSHOTS=docs/ux/screens PW_DIR=… node scripts/e2e-agent-rename.mjs   # also writes agent-rename-{hover,field}-{dark,light}.png
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'apps/web/dist');
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const shots = process.env.AGORA_SCREENSHOTS ? resolve(root, process.env.AGORA_SCREENSHOTS) : '';
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm' };

const server = createServer((req, res) => {
  const path = join(dist, !req.url || req.url === '/' ? 'index.html' : req.url.split('?')[0]);
  if (!existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const base = `http://127.0.0.1:${server.address().port}/#/demo`;

let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
if (shots) mkdirSync(shots, { recursive: true });

const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
try {
  for (const scheme of ['dark', 'light']) {
    const context = await browser.newContext({ colorScheme: scheme, viewport: { width: 1280, height: 800 } });
    // The demo's approval would open over the window; the attention layer is not what this checks.
    await context.addInitScript(() => { try { localStorage.setItem('agora.attention.mode', 'off'); } catch { /* the check runs without it */ } });
    const page = await context.newPage();
    await page.goto(base);
    const row = page.locator('[data-agent-row]').first();
    await row.waitFor();
    const id = await row.getAttribute('data-agent-row');
    const auto = (await row.locator('[data-agent-name]').textContent()) ?? '';
    const pencil = page.locator(`[data-agent-rename="${id}"]`);
    const shown = async () => Number(await pencil.evaluate((el) => getComputedStyle(el.parentElement).opacity));
    const before = (await row.boundingBox()).height;

    check((await shown()) === 0, `${scheme}: the pencil is hidden until the row is hovered`);
    await row.hover();
    await page.waitForTimeout(250);
    check((await shown()) === 1, `${scheme}: hovering the row shows the pencil`);
    check((await row.boundingBox()).height === before, `${scheme}: the row stays one line with the pencil`);
    if (shots) await page.locator('[data-region="sidebar"]').screenshot({ path: join(shots, `agent-rename-hover-${scheme}.png`) });

    await pencil.click();
    const field = page.locator(`[data-agent-naming="${id}"] input`);
    check(await field.evaluate((el) => document.activeElement === el && el.selectionStart === 0 && el.selectionEnd === el.value.length), `${scheme}: the field has the focus and the name selected`);
    await field.fill('Backend fixer');
    if (shots) await page.locator('[data-region="sidebar"]').screenshot({ path: join(shots, `agent-rename-field-${scheme}.png`) });
    await field.press('Enter');
    const name = page.locator(`[data-agent-row="${id}"] [data-agent-name]`);
    check((await name.textContent()) === 'Backend fixer', `${scheme}: Enter keeps the new name in the row`);
    check(await page.evaluate(() => document.activeElement?.hasAttribute('data-agent-row')), `${scheme}: the focus goes back to the row`);
    await page.locator(`[data-agent-row="${id}"]`).click();
    check(await page.locator('[role="tab"]', { hasText: 'Backend fixer' }).count() > 0, `${scheme}: the agent's tab shows the new name`);

    await page.reload();
    await page.locator(`[data-agent-row="${id}"]`).waitFor();
    check((await name.textContent()) === 'Backend fixer', `${scheme}: the name survives a reload`);

    await page.locator(`[data-agent-row="${id}"]`).focus();
    await page.keyboard.press('F2');
    await field.fill('');
    await field.press('Enter');
    check((await name.textContent()) === auto, `${scheme}: an empty name gives back "${auto}"`);
    await context.close();
  }
} finally {
  await browser.close();
  server.close();
}
console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
process.exit(failed ? 1 : 0);
