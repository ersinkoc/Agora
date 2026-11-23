// Real-browser check for the Agents list, the pane tabs and bars, and the file rows without bars (ADR-0102, redesign PR 7), on the built
// demo window (#/demo), in dark and light:
//   1. No bar on one side. Every Agents-list row, every tab, every rail icon and every file-manager and Files row, with everything inside
//      it and its ::before and ::after, has no border wider on one side and no inset shadow offset to the inline start or end. Rows are
//      checked in the states that had a bar: the row in front (aria-current), the chosen rail icon, the chosen file row and folder, and a
//      file row with keyboard focus (:focus-visible, reached with the arrow keys). A tab keeps its 2 px top rule (an ordinary tab affordance).
//   2. Who an agent is, in a narrow pane. With two panes about 220 px wide, the tab in front and the bar of a pane without tabs each show the
//      agent's avatar whole and inside the bar, and either at least 48 px of the name or (below the threshold) no name at all, kept as
//      the accessible name and the tooltip: never a sliver of letters. Split and maximize have folded into the ⋯ menu. The agent's header
//      keeps the folder readable (the machine and the branch leave it).
//   3. Keyboard selection still works: Enter on an Agents-list row puts it in front, the arrow keys move between tabs, and the file manager's
//      arrows move the focus from row to row.
//
//   cd apps/web && npm ci && npm run gen && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-explorer-no-bars.mjs
//   AGORA_SCREENSHOTS=docs/ux/screens PW_DIR=… node scripts/e2e-explorer-no-bars.mjs   # also writes explorer-redesign-{desktop,narrow,files}-{dark,light}.png
//
// A phone (narrower than 700 px) draws its own single column with no Agents list, tabs or pane bars (phone.md), so there is nothing of
// this change to check there.
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
if (shots) mkdirSync(shots, { recursive: true });

// In the page: each root and every element under it, with its two pseudo-elements. `tabs` may keep a top or bottom rule, never a side one.
function inspect(selector) {
  const problems = [];
  const px = (v) => Number.parseFloat(v) || 0;
  const name = (el) => `${el.tagName.toLowerCase()}.${[...el.classList].join('.')}${el.dataset.path ? `[${el.dataset.path}]` : ''}`;
  const roots = [...document.querySelectorAll(selector)];
  for (const r of roots) {
    const tab = r.getAttribute('role') === 'tab';
    for (const el of [r, ...r.querySelectorAll('*')]) {
      for (const pseudo of [null, '::before', '::after']) {
        const s = getComputedStyle(el, pseudo);
        if (pseudo && (s.content === 'none' || s.content === 'normal')) continue;
        const side = (k) => (s[`border${k}Style`] === 'none' ? 0 : px(s[`border${k}Width`]));
        const [left, right] = [side('Left'), side('Right')];
        // A border on one side only, and wider than a hairline, is a bar (a tab's 1 px divider on its end edge is not).
        if (Math.abs(left - right) > 1) problems.push(`${name(el)}${pseudo ?? ''}: border left ${left} right ${right}`);
        for (const part of s.boxShadow === 'none' ? [] : s.boxShadow.split(/,(?![^(]*\))/)) {
          if (!part.includes('inset')) continue;
          const [x, y] = (part.replace(/rgba?\([^)]*\)|color\([^)]*\)|oklab\([^)]*\)|oklch\([^)]*\)/g, '').match(/-?[\d.]+px/g) ?? []).map(px);
          if (x || (y && !tab)) problems.push(`${name(el)}${pseudo ?? ''}: inset shadow ${part.trim()}`);
        }
      }
    }
  }
  return { count: roots.length, problems };
}

// In the page: what each pane bar shows of its agent.
function identity() {
  const out = [];
  const visible = (el, box) => { const r = el.getBoundingClientRect(); return r.width >= 16 && r.left >= box.left - 0.5 && r.right <= box.right + 0.5; };
  const hidden = (el) => { const s = getComputedStyle(el); return s.position === 'absolute' && (s.clipPath.includes('inset(50%)') || el.getBoundingClientRect().width <= 1); };
  for (const bar of document.querySelectorAll('.pane-bar')) {
    const box = bar.getBoundingClientRect();
    const strip = bar.querySelector('.tab-strip');
    const holder = strip ? strip.querySelector('[role=tab][aria-selected=true]') : bar;
    const avatar = holder?.querySelector('.kit-avatar');
    if (!holder || !avatar) continue; // a pane that shows no agent (Home, an empty pane)
    const label = holder.querySelector('[data-pane-name]');
    const nameWidth = Math.round(label.getBoundingClientRect().width);
    const tip = (strip ? holder.title : avatar.closest('[title]')?.title) ?? '';
    out.push({
      width: Math.round(box.width), avatar: visible(avatar, strip ? strip.getBoundingClientRect() : box), name: label.textContent,
      shown: hidden(label) ? 0 : nameWidth, hiddenName: hidden(label), tip,
      folded: [...bar.querySelectorAll('[data-pane-tool]')].every((b) => getComputedStyle(b).display === 'none'),
    });
  }
  const heads = [...document.querySelectorAll('.agent-body')].map((b) => {
    const folder = b.querySelector('[data-crumb="machine"] ~ span.truncate, [data-crumb="machine"] + svg + span');
    return { width: Math.round(b.getBoundingClientRect().width), crumbs: [...b.querySelectorAll('[data-crumb]')].filter((c) => getComputedStyle(c).display !== 'none').length, folder: Math.round(folder?.getBoundingClientRect().width ?? 0) };
  });
  return { bars: out, heads };
}

const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
let failed = 0;
const check = (ok, message, detail = '') => { if (ok) console.log(`ok   ${message}`); else { failed++; console.log(`FAIL ${message}${detail ? `\n  ${detail}` : ''}`); } };
const open = async (viewport, scheme) => {
  const ctx = await browser.newContext({ viewport, colorScheme: scheme, reducedMotion: 'reduce' });
  await ctx.addInitScript(() => { try { localStorage.setItem('agora.attention.mode', 'off'); } catch { /* the check runs without it */ } });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => check(false, `no page error (${e.message})`));
  await page.goto(base);
  await page.waitForSelector('[data-agent-row]');
  await page.waitForTimeout(600);
  return { ctx, page };
};
const palette = async (page, text) => {
  await page.keyboard.press('Control+k');
  await page.keyboard.type(text);
  await page.waitForTimeout(150);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(600);
};

for (const scheme of ['dark', 'light']) {
  // 1 and 3: the wide window.
  {
    const { ctx, page } = await open({ width: 1440, height: 900 }, scheme);
    const rows = page.locator('[data-agent-row]');
    await rows.nth(1).focus();
    await page.keyboard.press('Enter'); // keyboard: the row opens and is in front
    await page.waitForTimeout(300);
    check(await rows.nth(1).getAttribute('aria-current') === 'true', `${scheme}: Enter on an Agents-list row puts it in front`);
    await rows.nth(3).click();
    await page.waitForTimeout(300);
    const front = page.locator('.tab-strip [role=tab][aria-selected=true]').first();
    const before = await front.getAttribute('id');
    await front.focus();
    await page.keyboard.press('ArrowLeft');
    await page.waitForTimeout(200);
    const after = await page.locator('.tab-strip [role=tab][aria-selected=true]').first().getAttribute('id');
    check(before !== after && await page.evaluate(() => document.activeElement?.getAttribute('aria-selected')) === 'true', `${scheme}: the arrow keys move between tabs (${before} → ${after})`);
    await page.locator('.tab-strip [role=tab]:not([data-cut])').filter({ has: page.locator('.kit-avatar') }).last().click();
    await page.waitForTimeout(200);
    if (shots) await page.screenshot({ path: join(shots, `explorer-redesign-desktop-${scheme}.png`), animations: 'disabled' });
    const r = await page.evaluate(inspect, '[data-agent-row], aside .ide-row, .tab-strip [role=tab], .rail__btn');
    check(r.count >= 15 && r.problems.length === 0, `${scheme}: ${r.count} Agents-list rows, tabs and rail icons have no bar on one side`, r.problems.join('\n  '));

    // The file manager and Files, beside the agent in front (the demo's files are codex · rose's).
    await page.getByRole('button', { name: /codex · rose/ }).first().click();
    await page.waitForTimeout(400);
    await palette(page, 'Open the File manager');
    const grid = page.locator('[role=grid] [role=row][data-path]');
    await grid.first().waitFor({ timeout: 5000 });
    await grid.first().focus();
    await page.keyboard.press('ArrowDown');
    await page.waitForTimeout(150);
    const moved = await page.evaluate(() => ({ path: document.activeElement?.getAttribute('data-path'), ring: document.activeElement?.matches(':focus-visible') && getComputedStyle(document.activeElement).boxShadow }));
    check(!!moved.path && moved.path !== await grid.first().getAttribute('data-path') && / 0px 0px 0px 2px inset/.test(String(moved.ring)), `${scheme}: the file manager's arrows move the focus, drawn as a ring (${moved.path}: ${moved.ring})`);
    await palette(page, 'Open Files beside');
    const src = page.locator('[role=tree] [role=treeitem]', { hasText: /^src$/ }).last();
    if (await src.count()) { await src.click(); await page.waitForTimeout(300); }
    const auth = page.locator('[role=tree] [role=treeitem]', { hasText: 'auth.rs' }).last();
    if (await auth.count()) { await auth.click(); await page.waitForTimeout(600); }
    await grid.nth(1).focus();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowUp');
    await page.waitForTimeout(150);
    if (shots) await page.screenshot({ path: join(shots, `explorer-redesign-files-${scheme}.png`), animations: 'disabled' });
    const f = await page.evaluate(inspect, '.fm-row, [role=tree] .ide-row, [role=tablist] .ide-tab');
    const chosen = await page.evaluate(() => document.querySelectorAll('.fm-row[aria-selected=true], [role=tree] .ide-row[aria-current=true]').length);
    check(f.count >= 6 && chosen >= 2 && f.problems.length === 0, `${scheme}: ${f.count} file rows and file tabs (${chosen} chosen, one with keyboard focus) have no bar on one side`, f.problems.join('\n  '));
    await ctx.close();
  }
  // 2: two narrow panes.
  {
    const { ctx, page } = await open({ width: 1100, height: 800 }, scheme);
    for (const n of [1, 3]) { await page.locator('[data-agent-row]').nth(n).click(); await page.waitForTimeout(250); }
    await page.locator('.pane-bar [data-pane-tool="split"]').first().click();
    await page.waitForTimeout(300);
    await page.getByRole('button', { name: /claude · teal/ }).last().click();
    await page.waitForTimeout(400);
    if (shots) await page.screenshot({ path: join(shots, `explorer-redesign-narrow-${scheme}.png`), animations: 'disabled' });
    const { bars, heads } = await page.evaluate(identity);
    check(bars.length >= 2, `${scheme}: ${bars.length} narrow pane bars show an agent`, JSON.stringify(bars));
    for (const b of bars) {
      const named = b.shown >= 48 || (b.hiddenName && b.tip.includes(b.name));
      check(b.avatar && named && b.folded, `${scheme}: a ${b.width} px pane bar keeps the avatar and ${b.hiddenName ? 'the name as its tooltip' : `${b.shown} px of "${b.name}"`}, its split and maximize folded into ⋯`, JSON.stringify(b));
    }
    for (const h of heads) check(h.crumbs === 0 && h.folder >= 24, `${scheme}: a ${h.width} px agent header shows its folder (${h.folder} px), not single letters`, JSON.stringify(h));
    const r = await page.evaluate(inspect, '.tab-strip [role=tab]');
    check(r.problems.length === 0, `${scheme}: narrow tabs and bars have no bar on one side`, r.problems.join('\n  '));
    await ctx.close();
  }
}
await browser.close();
server.close();
process.exit(failed ? 1 : 0);
