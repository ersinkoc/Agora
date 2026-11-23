// #1073: on a phone (390 × 844, a touch screen) every control inside a chat entry takes a finger: its hit area is at least --tap
// (44 px) in both directions, though the file chips and an inline link keep their drawn size. Measured where it counts, in the real
// browser: `elementFromPoint` along the lines through each control, so a hit area that is clipped, covered or never there fails. Also:
// no control's hit area covers another's own drawn box, the text around a link flows as it does with a mouse, and nothing adds scroll.
// Runs over the component gallery (both chats: the file chips, the "redirect notes" link, tool rows, output and permission records), no hub.
//
//   cd apps/web && npm ci && npm run gen && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-chat-touch.mjs [screenshot dir]
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'apps/web/dist');
const shots = process.argv[2];
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm' };
const TAP = 44;

const server = createServer((req, res) => {
  const path = join(dist, !req.url || req.url === '/' ? 'index.html' : req.url.split('?')[0]);
  if (!existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const base = `http://127.0.0.1:${server.address().port}/`;

const failures = [];
const check = (ok, what) => { if (!ok) failures.push(what); console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); };

// Every control a finger can press inside the chat logs, drawn (a control in a closed fold is laid out but not drawn), each given an
// index to find it again.
const CONTROLS = '[role=log] a[href], [role=log] button, [role=log] summary';
async function mark(page) {
  return page.evaluate((sel) => [...document.querySelectorAll(sel)].filter((el) => el.checkVisibility({ visibilityProperty: true, opacityProperty: true }))
    .map((el, i) => { el.setAttribute('data-touch-probe', String(i)); return { i, what: `${el.tagName.toLowerCase()} "${(el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 40)}"` }; }), CONTROLS);
}

// One control's hit area, measured: brought to the middle of the screen, then the points on the lines through the middle of its first
// box (a link that wraps is measured on each of its pieces), one pixel apart, that land on it (Chromium counts both edges, so a box reads
// a pixel more than its size at most). Also whether every point of its own drawn box lands on it (nothing else's hit area covers it).
async function measure(page, i) {
  return page.evaluate((i) => {
    const el = document.querySelector(`[data-touch-probe="${i}"]`);
    el.scrollIntoView({ block: 'center', inline: 'nearest' });
    // "Jump to latest" floats over the bottom of the log while it is scrolled up, by design: it is the log's, not an entry's, and it
    // comes and goes with the scrolling this does, so it is set aside while measuring.
    for (const j of document.querySelectorAll('.terminal-jump')) j.style.visibility = 'hidden';
    const on = (x, y) => { const hit = document.elementFromPoint(x, y); return !!hit && el.contains(hit); };
    const pieces = [...el.getClientRects()].filter((r) => r.width > 0 && r.height > 0);
    const spans = pieces.map((r) => {
      const cx = Math.floor(r.left + r.width / 2) + 0.5;
      const cy = Math.floor(r.top + r.height / 2) + 0.5;
      let h = 0; let w = 0;
      for (let d = 0; d < 80; d++) { if (on(cx, cy - d)) h++; else break; }
      for (let d = 1; d < 80; d++) { if (on(cx, cy + d)) h++; else break; }
      for (let d = 0; d < 400; d++) { if (on(cx - d, cy)) w++; else break; }
      for (let d = 1; d < 400; d++) { if (on(cx + d, cy)) w++; else break; }
      return { w, h, drawn: { w: Math.round(r.width), h: Math.round(r.height) } };
    });
    const covered = [];
    for (const r of pieces) {
      for (let y = Math.ceil(r.top) + 1; y < r.bottom - 1; y += 3) {
        for (let x = Math.ceil(r.left) + 1; x < r.right - 1; x += 3) {
          const hit = document.elementFromPoint(x, y);
          const other = hit && !el.contains(hit) ? hit.closest('a[href], button, summary') : null;
          if (other) covered.push((other.getAttribute('aria-label') || other.textContent || '').trim().slice(0, 30));
        }
      }
    }
    return { spans, covered: [...new Set(covered)] };
  }, i);
}

// What must not move: the height of every paragraph, list and table in a reply, and no added scroll range anywhere in the chats.
async function layout(page) {
  return page.evaluate(() => ({
    text: [...document.querySelectorAll('[data-role=agent] p, [data-role=agent] li, [data-role=agent] td')].map((e) => Math.round(e.getBoundingClientRect().height * 10)),
    page: document.scrollingElement.scrollWidth - document.scrollingElement.clientWidth,
    // A wide table scrolls sideways in its own box with a mouse too: what must hold is that touch adds no scroll range to anything.
    sideways: [...document.querySelectorAll('[role=log], [role=log] .overflow-x-auto, [role=log] pre, [data-files-changed]')].map((e) => e.scrollWidth - e.clientWidth),
    tables: [...document.querySelectorAll('[role=log] .overflow-x-auto, [role=log] pre')].map((e) => e.scrollHeight - e.clientHeight),
  }));
}

const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox', '--font-render-hinting=none'] });
for (const scheme of ['dark', 'light']) {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: scheme, reducedMotion: 'reduce' });
  const page = await ctx.newPage();
  await page.goto(`${base}#/gallery`);
  await page.waitForSelector('[data-files-changed]');
  await page.evaluate(() => document.fonts.ready);
  const cdp = await ctx.newCDPSession(page);
  if (scheme === 'dark') {
    // #1082: the gallery follows the window down to 300 px (every section wraps or scrolls inside itself), so the width search below
    // reaches the chats' narrow layouts instead of stopping at the widest section.
    await page.setViewportSize({ width: 300, height: 844 });
    await page.waitForTimeout(100);
    check(await page.evaluate(() => document.scrollingElement.scrollWidth - document.scrollingElement.clientWidth) === 0, 'at 300 px with a mouse the gallery page has no sideways scroll');
    await page.setViewportSize({ width: 390, height: 844 });
    await page.waitForTimeout(100);
  }
  const mouse = await layout(page);
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
  check(await page.evaluate(() => matchMedia('(pointer: coarse)').matches), `${scheme}: the browser reports a touch screen`);
  await page.waitForTimeout(300);

  if (scheme === 'dark') {
    const touch = await layout(page);
    check(JSON.stringify(touch.text) === JSON.stringify(mouse.text), 'the text of every reply (paragraphs, lists, table cells) flows as it does with a mouse: no height changed');
    check(touch.page === 0 && JSON.stringify(touch.sideways) === JSON.stringify(mouse.sideways), 'touch adds no sideways scroll to the page or to anything in the chats');
    check(JSON.stringify(touch.tables) === JSON.stringify(mouse.tables), 'touch adds no vertical scroll to a table or a code block in a reply');

    const list = await mark(page);
    const kinds = { chip: 0, link: 0 };
    for (const c of list) {
      const m = await measure(page, c.i);
      const isChip = c.what.startsWith('button') && (await page.$eval(`[data-touch-probe="${c.i}"]`, (e) => !!e.closest('[data-files-changed] ul')));
      const isLink = c.what.startsWith('a ');
      if (isChip) kinds.chip++;
      if (isLink) kinds.link++;
      m.spans.forEach((s, k) => check(s.w >= TAP && s.h >= TAP, `${c.what}${m.spans.length > 1 ? ` (piece ${k + 1})` : ''}: hit area ${s.w}×${s.h} (drawn ${s.drawn.w}×${s.drawn.h}) is at least ${TAP}×${TAP}`));
      check(m.covered.length === 0, `${c.what}: no other control's hit area covers it${m.covered.length ? ` (covered by ${m.covered.join(', ')})` : ''}`);
      if (isChip) check(m.spans[0].drawn.h <= 26, `${c.what}: the chip keeps its drawn size (${m.spans[0].drawn.h} px tall)`);
    }
    check(kinds.chip >= 2, `the gallery's chat has file chips (${kinds.chip})`);
    check(list.some((c) => c.what.includes('redirect notes')), 'the gallery\'s chat has the "redirect notes" link');

    // The link on two lines: a width where it wraps, then each of its pieces measured.
    let wrapped = false;
    for (let w = 300; w <= 430 && !wrapped; w += 2) {
      await page.setViewportSize({ width: w, height: 844 });
      const pieces = await page.evaluate(() => { const a = [...document.querySelectorAll('[role=log] a[href]')].find((e) => e.textContent === 'redirect notes'); return a ? a.getClientRects().length : 0; });
      if (pieces < 2) continue;
      wrapped = true;
      const i = await page.evaluate(() => [...document.querySelectorAll('[data-touch-probe]')].find((e) => e.textContent === 'redirect notes').getAttribute('data-touch-probe'));
      const m = await measure(page, i);
      m.spans.forEach((s, k) => check(s.h >= TAP, `at ${w} px the link wraps: piece ${k + 1} is ${s.h} px tall to a finger (drawn ${s.drawn.h})`));
      // What touch must not do at this width: add sideways scroll to the page or to anything in the chats, compared with the same page
      // and width with a mouse.
      await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: false });
      await page.waitForTimeout(100);
      const mouseAt = await layout(page);
      await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
      await page.waitForTimeout(100);
      const touchAt = await layout(page);
      check(touchAt.page === mouseAt.page && JSON.stringify(touchAt.sideways) === JSON.stringify(mouseAt.sideways),
        `at ${w} px touch adds no sideways scroll to the page or to the chats (page ${touchAt.page} px, ${mouseAt.page} px with a mouse)`);
    }
    check(wrapped, 'a width between 300 and 430 px wraps the "redirect notes" link over two lines');
    await page.setViewportSize({ width: 390, height: 844 });
  }

  if (shots) {
    mkdirSync(shots, { recursive: true });
    // The gallery draws everything twice, in its dark and its light panel: each theme's shots come from its own panel.
    for (const [name, sel] of [['files', '[data-files-changed]'], ['link', '[role=log] a[href]']]) {
      await page.evaluate((s) => document.querySelector(s).scrollIntoView({ block: 'center' }), `.theme-panel[data-theme=${scheme}] ${sel}`);
      await page.waitForTimeout(100);
      await page.screenshot({ path: join(shots, `chat-touch-${name}-${scheme}.png`) });
    }
  }
  await ctx.close();
}
// What the gallery's chats do not have, added to its first chat. A link in the last row of a table in a reply (the gallery's table has
// none, so one of its links is copied there): its hit area reaches past the row, and that must not make the table scroll up and down (it
// may reach past the table's box, which clips it).
{
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, reducedMotion: 'reduce' });
  const page = await ctx.newPage();
  await page.goto(`${base}#/gallery`);
  await page.waitForSelector('[data-files-changed]');
  await (await ctx.newCDPSession(page)).send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
  const extra = await page.evaluate(() => {
    const box = document.querySelector('[role=log] .overflow-x-auto');
    box.querySelector('tr:last-child td:last-child').append(' ', document.querySelector('[role=log] a[href]').cloneNode(true));
    const y = getComputedStyle(box).overflowY;
    return y === 'hidden' || y === 'clip' ? 0 : box.scrollHeight - box.clientHeight;
  });
  check(extra === 0, `a link in a table's last row does not make the table scroll up and down (${extra} px)`);

  // Links on lines next to each other (a list of sources, a sentence of links; the gallery has neither, so its link is copied): each
  // reaches into the next line, and a tap on a link's own words must still open that link, never its neighbour.
  const words = await page.evaluate(() => {
    const link = document.querySelector('[role=log] a[href]');
    const reply = link.closest('[data-role=agent]').querySelector(':scope > .grid');
    const copy = (text) => { const a = link.cloneNode(true); a.firstChild.textContent = text; return a; };
    const ul = document.createElement('ul');
    ul.className = 'm-0 grid list-disc gap-0.5 ps-5 marker:text-muted';
    for (let i = 0; i < 3; i++) { const li = document.createElement('li'); li.append(copy(`source ${i}`)); ul.append(li); }
    const p = document.createElement('p');
    p.className = 'm-0 break-words whitespace-pre-wrap';
    for (let i = 0; i < 6; i++) p.append(copy(`a longer link ${i}`), ' ');
    reply.append(ul, p);
    const out = [];
    for (const a of reply.querySelectorAll('a[href]')) {
      a.scrollIntoView({ block: 'center' });
      const words = document.createRange();
      words.selectNodeContents(a);
      let lost = 0;
      for (const g of words.getClientRects()) {
        for (let y = g.top + 1; y < g.bottom - 1; y += 2) for (let x = g.left + 1; x < g.right - 1; x += 3) if (!a.contains(document.elementFromPoint(x, y))) lost++;
      }
      out.push([a.textContent, lost]);
    }
    return out;
  });
  check(words.length >= 9 && words.every(([, lost]) => lost === 0), `a tap on a link's own words opens that link, with links on the lines above and below (${words.filter(([, n]) => n).map(([w, n]) => `${w}: ${n}`).join(', ') || 'none lost'})`);

  // The chips with the Review button (the full window passes one; the gallery does not, so one is added as FilesChanged draws it).
  const ids = await page.evaluate(() => {
    const box = document.querySelector('[data-files-changed]');
    const review = document.createElement('button');
    review.className = 'button px-3 py-1 text-[0.8125rem]';
    review.textContent = 'Review changes';
    box.append(review);
    return [...box.querySelectorAll('button')].map((b, i) => { b.setAttribute('data-touch-probe', `files-${i}`); return `files-${i}`; });
  });
  for (const id of ids) {
    const m = await measure(page, id);
    const what = await page.$eval(`[data-touch-probe="${id}"]`, (e) => e.textContent.trim());
    check(m.spans[0].h >= TAP && m.covered.length === 0, `with the Review button: "${what}" is ${m.spans[0].w}×${m.spans[0].h} to a finger and nothing covers it${m.covered.length ? ` (covered by ${m.covered.join(', ')})` : ''}`);
  }
  await ctx.close();
}
await browser.close();
server.close();
if (failures.length) { console.log(`\n${failures.length} failed`); process.exit(1); }
console.log('\nPASS: every control in a chat entry has a 44 px hit area on a touch phone; the chips and the link keep their size and the text its flow.');
