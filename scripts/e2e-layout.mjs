// End-to-end check of the layouts (ADR-0068) in a real browser: four shell agents in a 2 × 2 grid, a divider dragged and moved
// with the keyboard (the terminals are not re-created), a pane maximized and restored (hidden panes drop their terminal stream),
// a pane dragged onto another as a tab, a flood in one pane while typing in another, the layout saved by name and found again
// after a reload, and a pane that follows what needs you (a chat agent asking permission, then answered).
//
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm ci && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-layout.mjs
//   AGORA_SCREENSHOTS=docs/screenshots node scripts/e2e-layout.mjs   # also writes adr68-*.png in both themes
//
// Needs git, sh and a Chromium (CHROME=<path>, default: Playwright's cache). Uses a throw-away folder; changes nothing else.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';

let failed = false;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed = true; };
let stack;

try {
  stack = await startStack({ viewport: { width: 1600, height: 1000 } });
  const { page, start, errors, origin } = stack;
  page.setDefaultTimeout(15000);
  const shots = process.env.AGORA_SCREENSHOTS;
  const shoot = async (name) => {
    if (!shots) return;
    mkdirSync(shots, { recursive: true });
    for (const theme of ['dark', 'light']) {
      await page.evaluate((t) => { document.documentElement.dataset.theme = t; window.dispatchEvent(new Event('agora-theme')); }, theme);
      await page.waitForTimeout(150);
      await page.screenshot({ path: join(shots, `adr68-${name}-${theme}.png`), animations: 'disabled' });
    }
    await page.evaluate(() => { delete document.documentElement.dataset.theme; window.dispatchEvent(new Event('agora-theme')); });
  };

  for (let i = 0; i < 4; i++) await start('Shell');
  await page.goto(`${origin}/#/`);
  const terminals = () => page.locator('[data-pane] .xterm').count();
  // Untouched, the window opens the first four agents as tabs of one pane, as before layouts.
  await page.waitForFunction(() => document.querySelectorAll('main [role=tab]').length === 4);
  check((await terminals()) === 1, 'four agents open as tabs of one pane; only the tab in front holds a terminal');

  const layoutMenu = async (item) => {
    await page.getByRole('button', { name: 'Layout', exact: true }).click();
    await page.getByRole('menuitem', { name: item, exact: true }).click();
  };
  await layoutMenu('Four (2 × 2)');
  await page.waitForFunction(() => document.querySelectorAll('[data-pane] .xterm').length === 4);
  const separators = page.locator('[data-layout-root] [role=separator]');
  check((await separators.count()) === 3, '2 × 2 shows four terminals and three dividers');
  check((await page.locator('[data-pane][role=region][aria-label]').count()) === 4, 'every pane is a labelled region');

  // Panes in reading order (the page keeps them in a fixed order of their own, so moving one never re-creates it).
  const ordered = () => page.evaluate(() => [...document.querySelectorAll('[data-pane]')].map((p) => [p.id, p.getBoundingClientRect()])
    .sort(([, a], [, b]) => a.top - b.top || a.left - b.left).map(([id]) => id));
  const order = await ordered();
  const panes = { nth: (i) => page.locator(`#${order[i]}`), first: () => page.locator(`#${order[0]}`) };
  // Each shell says which pane it is, so the screenshot shows four live terminals.
  for (let i = 0; i < 4; i++) {
    await panes.nth(i).locator('.xterm').click();
    await page.waitForFunction((id) => /\$\s*$/.test((document.querySelector(`#${id} .xterm-rows`)?.innerText ?? '').trim()), order[i], { timeout: 30000 }).catch(() => {});
    await page.keyboard.type(`echo pane-${i + 1}\n`, { delay: 5 });
  }
  await page.waitForFunction((ids) => ids.every((id, i) => (document.querySelector(`#${id} .xterm-rows`)?.innerText ?? '').split(`pane-${i + 1}`).length > 2), order, { timeout: 20000 }).catch(() => {});
  await shoot('2x2');

  // Drag the first row's divider: the panes resize, the terminals stay the same elements (not re-created).
  await page.evaluate(() => document.querySelectorAll('[data-pane] .xterm').forEach((x, i) => { x.dataset.mark = String(i); }));
  const vertical = page.locator('[data-layout-root] [role=separator][aria-orientation=vertical]').first();
  const before = Number(await vertical.getAttribute('aria-valuenow'));
  const box = await vertical.boundingBox();
  const widthBefore = (await panes.nth(0).boundingBox()).width;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 160, box.y + box.height / 2, { steps: 8 });
  await page.mouse.up();
  const after = Number(await vertical.getAttribute('aria-valuenow'));
  const widths = await Promise.all([0, 1].map(async (i) => (await panes.nth(i).boundingBox()).width));
  check(after > before + 5 && Math.abs(Math.max(...widths) - widthBefore - 160) < 40, `dragging a divider resizes its panes (${before}% → ${after}%)`);
  check(await page.evaluate(() => [...document.querySelectorAll('[data-pane] .xterm')].every((x) => x.dataset.mark !== undefined)), 'the terminals were not re-created by the resize');
  await vertical.focus();
  await page.keyboard.press('ArrowLeft');
  await page.keyboard.press('ArrowLeft');
  const keyed = Number(await vertical.getAttribute('aria-valuenow'));
  check(keyed === after - 4 && (await page.evaluate(() => document.activeElement?.getAttribute('role'))) === 'separator', `a focused divider moves with the arrow keys and keeps focus (${after}% → ${keyed}%)`);

  // Maximize the first pane: one terminal on screen (the others closed their streams); restore brings the four back.
  await panes.first().click();
  await page.getByRole('button', { name: 'Maximize', exact: true }).first().click();
  await page.waitForFunction(() => document.querySelectorAll('[data-pane] .xterm').length === 1);
  check((await separators.count()) === 0, 'maximize shows one pane, without dividers; the hidden terminals are unmounted');
  await page.getByRole('button', { name: 'Restore', exact: true }).first().click();
  await page.waitForFunction(() => document.querySelectorAll('[data-pane] .xterm').length === 4);
  check(Number(await vertical.getAttribute('aria-valuenow')) === keyed, 'restore brings the grid back as it was');

  // A flood in one pane does not hold up typing in another.
  await panes.nth(0).locator('.xterm').click();
  await page.keyboard.type('seq 1 400000\n', { delay: 2 });
  await panes.nth(1).locator('.xterm').click();
  const typed = Date.now();
  await page.keyboard.type('echo typed-while-flooding\n', { delay: 2 });
  const echoed = await page.waitForFunction((id) => (document.querySelector(`#${id} .xterm-rows`)?.innerText ?? '').split('typed-while-flooding').length > 2, order[1], { timeout: 10000 }).then(() => true, () => false);
  check(echoed, `typing in one pane while another floods comes back (${Date.now() - typed} ms)`);

  // Save it by name, reload: the layout and the divider are as they were, and the name is listed.
  await layoutMenu('Save layout as…');
  await page.getByRole('dialog').locator('input').fill('Four shells');
  await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
  await page.reload();
  await page.waitForFunction(() => document.querySelectorAll('[data-pane] .xterm').length === 4, null, { timeout: 20000 });
  check(Number(await vertical.getAttribute('aria-valuenow')) === keyed, 'after a reload the 2 × 2 layout and its divider are back');
  await layoutMenu('Saved layouts…');
  check(await page.getByRole('dialog').getByText('Four shells', { exact: true }).isVisible(), 'the saved layout is listed by name');
  await page.getByRole('dialog').getByRole('button', { name: 'Close', exact: true }).click();

  // Drag the first pane's bar onto the middle of the second: it becomes a tab there.
  const bar = page.locator('[data-layout-root] [draggable=true]').first();
  const target = await panes.nth(1).boundingBox();
  const from = await bar.boundingBox();
  await page.mouse.move(from.x + 40, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + 80, from.y + from.height / 2, { steps: 4 });
  await page.waitForTimeout(100);
  await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 10 });
  await page.waitForTimeout(100);
  await page.mouse.move(target.x + target.width / 2 + 2, target.y + target.height / 2 + 2);
  await page.mouse.up();
  const tabbed = await page.waitForFunction(() => document.querySelectorAll('[data-layout-root] [role=tab]').length === 2, null, { timeout: 5000 }).then(() => true, () => false);
  if (!tabbed) {
    // Headless Chromium does not always start a native drag from synthetic pointer events; the keyboard path does the same move.
    console.log('(the pointer drag did not start in this browser; moving with the pane menu instead)');
    await page.getByRole('button', { name: /^Pane options/ }).first().click();
    await page.getByRole('menuitem', { name: 'Move to…' }).click();
    await page.getByRole('menuitem').nth(1).click();
    await page.getByRole('menuitem', { name: 'As a tab' }).click();
  }
  await page.waitForFunction(() => document.querySelectorAll('[data-layout-root] [role=tab]').length === 2);
  check((await terminals()) === 3, `a pane ${tabbed ? 'dragged' : 'moved'} onto another becomes a tab there; three terminals on screen`);

  // A chat agent asks permission: a pane that follows what needs you shows it, and lets go once it is answered.
  await start('Fake chat agent');
  const chatId = decodeURIComponent(page.url().split('#/agent/')[1]);
  await page.waitForSelector('form textarea');
  await page.fill('form textarea', 'ask');
  await page.click('button:has-text("Send")');
  await page.goto(`${origin}/#/`);
  await page.waitForFunction(() => document.querySelectorAll('[data-pane]').length >= 3);
  await page.getByRole('button', { name: /^Pane options/ }).last().click();
  await page.getByRole('menuitem', { name: 'Show…' }).click();
  await page.getByRole('menuitemradio', { name: 'What needs you next' }).click();
  await page.locator('[data-layout-root] .ide-chip', { hasText: 'Follows what needs you' }).waitFor();
  const shown = await page.waitForFunction((id) => !!document.querySelector(`[data-agent="${CSS.escape(id)}"]`), chatId, { timeout: 20000 }).then(() => true, () => false);
  check(shown, 'a pane set to follow what needs you shows the agent asking permission');
  await shoot('follow-attention');
  const allow = page.getByRole('button', { name: 'Allow', exact: true }).locator('visible=true');
  await allow.first().click();
  const calm = await page.waitForFunction((id) => !document.querySelector(`[data-agent="${CSS.escape(id)}"]`) && [...document.querySelectorAll('[data-pane]')].some((p) => /Nothing needs you/.test(p.textContent ?? '')), chatId, { timeout: 20000 }).then(() => true, () => false);
  check(calm, 'once answered, the pane moves on (nothing else needs you: it says so)');

  // A phone: no tiling, the panes are tabs of one group; the terminal in front gets the height, nothing is clipped at the edge.
  const fits = (w) => page.evaluate((width) => {
    const out = [...document.querySelectorAll('header button, header a, [data-layout-root] button, aside button')]
      // A tab strip that scrolls sideways may hold tabs beyond the edge: they are reachable by scrolling, not clipped.
      .filter((b) => !b.closest('[role=tablist]'))
      .filter((b) => { const r = b.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(b).visibility !== 'hidden' && (r.left < -1 || r.right > width + 1); })
      .map((b) => (b.getAttribute('aria-label') || b.textContent || '').trim().slice(0, 30));
    return { scroll: document.documentElement.scrollWidth, out };
  }, w);
  const keyButtons = ['Layout', 'Start an agent'];
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForFunction(() => document.querySelectorAll('[data-layout-root] [role=separator]').length === 0);
  check((await page.locator('[data-layout-root] [role=tab]').count()) >= 4 && (await terminals()) <= 1, 'on a phone the panes become tabs of one group, one on screen');
  await page.locator('[data-layout-root] [role=tab]').first().click();
  await page.waitForFunction(() => document.querySelectorAll('[data-pane] .xterm').length === 1);
  await page.waitForTimeout(300);
  const termH = (await page.locator('[data-pane] .xterm').first().boundingBox())?.height ?? 0;
  check(termH >= 0.4 * 844, `on a phone the terminal gets the height (${Math.round(termH)} px of 844)`);
  const phone = await fits(390);
  check(phone.scroll <= 390 && phone.out.length === 0, `on a phone nothing is wider than the screen${phone.out.length ? `: ${phone.out.join(', ')}` : ''} (page ${phone.scroll} px)`);
  for (const name of keyButtons) {
    const b = page.getByRole('button', { name, exact: true }).locator('visible=true').first();
    const r = await b.boundingBox().catch(() => null);
    check(!!r && r.x >= 0 && r.x + r.width <= 390, `on a phone "${name}" is within the screen`);
  }
  await shoot('phone');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.waitForTimeout(300);
  const desk = await fits(1440);
  check(desk.scroll <= 1440 && desk.out.length === 0, `at 1440 × 900 nothing is clipped${desk.out.length ? `: ${desk.out.join(', ')}` : ''}`);
  await page.setViewportSize({ width: 1600, height: 1000 });

  check((await page.locator('[role=alert]').count()) === 0, 'no error is shown');
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors[0]}` : ''}`);
} catch (e) {
  console.log('FAIL', e.message);
  if (process.env.E2E_DEBUG) await stack?.page.screenshot({ path: process.env.E2E_DEBUG }).catch(() => {});
  failed = true;
} finally {
  await stack?.close();
}
process.exit(failed ? 1 : 0);
