// Real-browser checks of the terminal window (docs/ux/terminal-fidelity.md). Start the lab first, from apps/web:
//   LAB_PORT=5231 npx vite --config ../../scripts/terminal-fidelity/vite.config.mjs      (serves http://127.0.0.1:5231/lab)
// then:  PW_DIR=<dir with node_modules/playwright-core> LAB_PORT=5231 node scripts/terminal-fidelity/run.mjs [shots-dir]
// CHROME=<path> picks the browser. Exits non-zero when a check fails.
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = createRequire(join(process.env.PW_DIR ?? process.cwd(), 'package.json'))('playwright-core');
const URL = `http://127.0.0.1:${process.env.LAB_PORT ?? 5199}/lab`;
const CHROME = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const shots = process.argv[2] ? resolve(process.argv[2]) : undefined;
if (shots) mkdirSync(shots, { recursive: true });

let failed = 0;
const check = (name, ok, detail = '') => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? `  [${detail}]` : ''}`); if (!ok) failed++; };
const J = JSON.stringify;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

const browser = await chromium.launch({ executablePath: CHROME, args: ['--no-sandbox'] });

// The checks read the screen's text from the page's own text drawing (gpu: false); the graphics-card drawing is a canvas.
async function open({ width = 900, height = 520, scheme = 'dark', dpr = 1, touch = false, prefs = { gpu: false } } = {}) {
  const ctx = await browser.newContext({ viewport: { width, height }, colorScheme: scheme, deviceScaleFactor: dpr, hasTouch: touch, isMobile: touch, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => { console.log('PAGE ERROR', e.message); failed++; });
  await page.addInitScript((p) => { try { if (!localStorage.getItem('agora.terminal.prefs')) localStorage.setItem('agora.terminal.prefs', JSON.stringify(p)); } catch {} }, prefs);
  await page.goto(URL);
  await page.waitForSelector('.xterm-screen');
  await page.waitForFunction(() => document.querySelector('.terminal')?.dataset.cols);
  const geo = async () => {
    const b = await page.locator('.xterm-screen').boundingBox();
    const cols = Number(await page.locator('[role=region].terminal').getAttribute('data-cols'));
    const rows = Number(await page.locator('[role=region].terminal').getAttribute('data-rows'));
    return { x: b.x, y: b.y, cols, rows, cw: b.width / cols, ch: b.height / rows, w: b.width, h: b.height };
  };
  const sent = () => page.evaluate(() => window.__sent.splice(0));
  const emit = (s, kind) => page.evaluate(([s, kind]) => window.__emit(s, kind), [s, kind]);
  const clip = () => page.evaluate(() => navigator.clipboard.readText());
  const top = () => page.evaluate(() => document.querySelector('.xterm-rows').children[0].textContent.trim());
  const cell = (g, col, row) => ({ x: g.x + (col - 0.5) * g.cw, y: g.y + (row - 0.5) * g.ch });
  return { ctx, page, geo, sent, emit, clip, top, cell };
}
const lines = (n, f = (i) => `line ${i} hello world foo-bar`) => Array.from({ length: n }, (_, i) => f(i)).join('\r\n');

// ---- 1. Mouse reporting to a real program on a PTY ----
{
  const t = await open();
  const { page } = t;
  const py = spawn('python3', [join(here, 'pty-run.py'), '111', '28', 'python3', join(here, 'mouse-echo.py')], { stdio: ['pipe', 'pipe', 'inherit', 'pipe'] });
  let screenText = '';
  await page.exposeFunction('__toPty', (s) => py.stdin.write(Buffer.from(s, 'latin1')));
  await page.exposeFunction('__ptyResize', (c, r) => py.stdio[3].write(`${c} ${r}\n`));
  await page.evaluate(() => { window.__onInput = (s) => window.__toPty(s); window.__onReport = (c, r) => window.__ptyResize(c, r); });
  py.stdout.on('data', (d) => { screenText += d.toString('latin1'); page.evaluate((b) => window.__emit(Uint8Array.from(atob(b), (c) => c.charCodeAt(0))), d.toString('base64')).catch(() => undefined); });
  await wait(900);
  const g = await t.geo();
  const gots = async () => { await wait(250); const rows = await page.evaluate(() => [...document.querySelectorAll('.xterm-rows > div')].map((r) => r.textContent)); return rows.filter((r) => r.startsWith('GOT')).map((r) => r.replace(/\s+$/, '')); };
  const last = async () => { const r = await gots(); return r[r.length - 1] ?? ''; };
  const c = t.cell(g, 10, 5);
  await page.mouse.click(c.x, c.y);
  let l = await gots();
  check('PTY program: click is reported in SGR (1006), press then release, at the right cell', l.some((x) => x.includes('<0;10;5M')) && l.some((x) => x.includes('<0;10;5m')), J(l.slice(-3)));
  await page.mouse.move(c.x, c.y);
  await page.mouse.wheel(0, -100);
  check('PTY program: wheel up arrives as button 64', (await last()).includes('<64;10;5M'), await last());
  await page.mouse.wheel(0, 100);
  check('PTY program: wheel down arrives as button 65', (await last()).includes('<65;10;5M'), await last());
  await page.mouse.move(c.x, c.y); await page.mouse.down();
  const d = t.cell(g, 20, 8);
  await page.mouse.move(d.x, d.y, { steps: 5 }); await page.mouse.up();
  l = await gots();
  check('PTY program: dragging reports motion with the button held (1002)', l.some((x) => /<32;\d+;\d+M/.test(x)) && l.some((x) => x.includes('<0;20;8m')), J(l.slice(-3)));
  await page.keyboard.down('Shift');
  const e = t.cell(g, 5, 1);
  const before = (await gots()).length;
  await page.mouse.move(e.x, e.y); await page.mouse.down(); await page.mouse.move(e.x + 8 * g.cw, e.y, { steps: 4 }); await page.mouse.up();
  await page.keyboard.up('Shift');
  const after = (await gots()).length;
  const sel = await page.evaluate(() => getSelection().toString());
  check('PTY program: Shift+drag selects text locally and the program hears nothing', after === before && sel.length > 0, `reports ${before}->${after}, selected ${J(sel)}`);
  await page.keyboard.press('Control+Shift+C');
  check('Ctrl+Shift+C copies that selection to the clipboard', (await t.clip()) === sel, J(await t.clip()));
  await page.mouse.click(c.x, c.y); await t.sent();
  await page.locator('body').click({ position: { x: 2, y: 2 }, force: true }).catch(() => undefined);
  await wait(150);
  check('PTY program: focus events (1004) reach it', (await gots()).some((x) => x.includes('[I') || x.includes('[O')));
  py.stdin.write('q'); await wait(200); py.kill();
  await t.ctx.close();
}

// ---- 2. Mouse encodings, the wheel, the alternate screen, selection, paste, keys ----
{
  const t = await open();
  const { page, emit, sent } = t;
  const g = await t.geo();
  // Older X10 form: bytes above 127 must stay single bytes (a column over 95).
  await emit('\x1b[?1000h');
  const far = t.cell(g, 100, 4);
  await page.mouse.click(far.x, far.y); await wait(80);
  const x10 = (await sent()).join('');
  const codes = [...x10].map((ch) => ch.charCodeAt(0));
  check('default (X10) mouse report is raw bytes: ESC [ M, button, column+32, row+32', J(codes.slice(0, 6)) === J([27, 91, 77, 32, 132, 36]), J(codes));
  await emit('\x1b[?1000l');

  // Wheel, no mouse mode: history scrolls three lines a notch, a trackpad by its swipe.
  await emit(lines(3000, (i) => `line ${i}`)); await wait(400);
  await page.mouse.move(g.x + 100, g.y + 60);
  const n = async () => Number((await t.top()).split(' ')[1]);
  let n0 = await n();
  await page.mouse.wheel(0, -100); await wait(300);
  check('wheel, no mouse mode: one notch scrolls 3 lines', n0 - (await n()) === 3, `${n0 - (await n())}`);
  await emit('\x1b[?1000h\x1b[?1006h'); await sent();
  await page.mouse.wheel(0, -100); await wait(300);
  check('mouse mode on: the wheel goes to the program as a mouse report', /^\x1b\[<64;/.test((await sent()).join('')));
  n0 = await n();
  await page.keyboard.down('Shift'); await page.mouse.wheel(0, -100); await page.keyboard.up('Shift'); await wait(300);
  check('mouse mode on: Shift+wheel scrolls the history 3 lines and sends nothing', n0 - (await n()) === 3 && (await sent()).length === 0, `${n0 - (await n())}`);
  await emit('\x1b[?1000l\x1b[?1006l');
  // Scrollback depth
  await page.mouse.click(g.x + 100, g.y + 60);
  for (let i = 0; i < 130; i++) await page.keyboard.press('Shift+PageUp');
  await wait(200);
  const oldest = await n();
  check('default history keeps 5000 lines (3000 printed are all there)', oldest === 0, `top line ${oldest}`);
  await page.keyboard.press('Shift+End'); await wait(150);

  // Scroll anchoring + jump to latest.
  await page.mouse.move(g.x + 100, g.y + 60);
  for (let i = 0; i < 4; i++) { await page.mouse.wheel(0, -100); await wait(120); }
  const anchored = await t.top();
  const hasJump = await page.locator('.terminal-jump').count();
  await emit(lines(60, (i) => `fresh ${i}`) + '\r\n'); await wait(250);
  check('new output while scrolled up does not move the view', (await t.top()) === anchored, `${anchored} -> ${await t.top()}`);
  check('a "Jump to latest" button shows while scrolled up', hasJump === 1 || (await page.locator('.terminal-jump').count()) === 1);
  if (shots) await page.screenshot({ path: join(shots, 'jump-to-latest.png') });
  await page.locator('.terminal-jump').click(); await wait(200);
  check('the button takes the view back to the latest output', (await page.locator('.terminal-jump').count()) === 0 && (await t.top()).startsWith('fresh'), await t.top());

  // Alternate screen without mouse: arrows.
  await sent();
  await emit('\x1b[?1049h\x1b[2J\x1b[Halt'); await wait(80);
  await page.mouse.move(g.x + 100, g.y + 60);
  await page.mouse.wheel(0, -100); await wait(150);
  check('alternate screen, no mouse mode: a notch up sends 3 arrow-up keys', J(await sent()) === J(['\x1b[A\x1b[A\x1b[A']));
  await emit('\x1b[?1h'); await page.mouse.wheel(0, 100); await wait(150);
  check('...as application cursor keys when the program asked for them (DECCKM)', J(await sent()) === J(['\x1bOB\x1bOB\x1bOB']));
  await emit('\x1b[?1l');
  await emit('\x1b[?1000h\x1b[?1006h');
  await page.mouse.wheel(0, -100); await wait(150);
  check('alternate screen with mouse mode: the wheel is a mouse report', /^\x1b\[<64;/.test((await sent()).join('')));
  await emit('\x1b[?1000l\x1b[?1006l\x1b[?1049l\x1b[2J\x1b[H');

  // Selection, copy, paste.
  await emit(lines(8, (i) => `row ${i} alpha beta gamma`)); await wait(100);
  const w = t.cell(g, 8, 4);
  await page.mouse.dblclick(w.x, w.y);
  const word = await page.evaluate(() => getSelection().toString());
  await page.mouse.click(w.x, w.y, { clickCount: 3 });
  const lineSel = await page.evaluate(() => getSelection().toString());
  check('double click selects a word, triple click the line', word === 'alpha' && lineSel.startsWith('row 3'), `${J(word)} ${J(lineSel)}`);
  await page.keyboard.down('Alt');
  const a1 = t.cell(g, 5, 2), a2 = t.cell(g, 9, 5);
  await page.mouse.move(a1.x, a1.y); await page.mouse.down(); await page.mouse.move(a2.x, a2.y, { steps: 4 }); await page.mouse.up();
  await page.keyboard.up('Alt');
  const rect = (await page.evaluate(() => getSelection().toString())).split('\n');
  check('Alt+drag selects a rectangle', rect.length >= 3 && rect.every((r) => r.length === rect[0].length && r.length >= 4), J(rect));
  await page.mouse.dblclick(w.x, w.y); await sent();
  await page.keyboard.press('Control+Shift+C'); await wait(100);
  check('Ctrl+Shift+C copies the selection and sends nothing to the program', (await t.clip()) === 'alpha' && (await sent()).length === 0, J(await t.clip()));
  await page.keyboard.press('Control+C');
  check('Ctrl+C still reaches the program (interrupt)', J(await sent()) === J(['\x03']));
  await page.evaluate(() => navigator.clipboard.writeText('one\ntwo'));
  await page.keyboard.press('Control+Shift+V'); await wait(100);
  check('Ctrl+Shift+V pastes as typed lines (\\r)', J(await sent()) === J(['one\rtwo']));
  await emit('\x1b[?2004h'); await page.keyboard.press('Control+Shift+V'); await wait(100);
  check('...and in bracketed paste (2004) when the program asked', J(await sent()) === J(['\x1b[200~one\rtwo\x1b[201~']));
  await emit('\x1b[?2004l');
  // Keys.
  await emit('\x1b[>1u'); await page.keyboard.press('Shift+Enter');
  check('CSI u keyboard: Shift+Enter is CSI 13;2 u once the program set flag 1', J(await sent()) === J(['\x1b[13;2u']));
  await emit('\x1b[?u'); await wait(60);
  check('...and the query is answered with the flags', J(await sent()) === J(['\x1b[?1u']));
  await emit('\x1b[<u'); await page.keyboard.press('Shift+Enter');
  check('...and popping them returns to a plain Enter', J(await sent()) === J(['\r']));
  await emit('\x1b[>4;2m'); await page.keyboard.press('Shift+Enter');
  check('modifyOtherKeys 2: Shift+Enter is CSI 27;2;13 ~', J(await sent()) === J(['\x1b[27;2;13~']));
  await emit('\x1b[>4;0m'); await page.keyboard.press('Control+ArrowLeft'); await page.keyboard.press('Alt+b');
  check('Ctrl+Left and Alt+b are the xterm forms', J(await sent()) === J(['\x1b[1;5D', '\x1bb']));

  // Titles, OSC 52, links, bell.
  const title0 = await page.title();
  await emit('\x1b]0;pwned\x07\x1b]2;pwned\x07');
  check('OSC 0/2 never reaches document.title', (await page.title()) === title0);
  await page.evaluate(() => navigator.clipboard.writeText('mine'));
  await emit(`\x1b]52;c;${Buffer.from('evil rm -rf').toString('base64')}\x07`); await wait(150);
  check('OSC 52 write is not applied by itself', (await t.clip()) === 'mine');
  check('...it is offered with Copy / Ignore', (await page.locator('.terminal-offer').count()) === 1);
  if (shots) await page.screenshot({ path: join(shots, 'clipboard-offer.png') });
  await page.locator('.terminal-offer button', { hasText: /^Copy$/ }).click(); await wait(100);
  check('...and copied only after Copy', (await t.clip()) === 'evil rm -rf');
  await emit('\x1b]52;c;?\x07'); await wait(100);
  check('OSC 52 read is never answered', (await sent()).length === 0);
  await emit('\x1b[2J\x1b[H\x1b]8;;javascript:alert(1)\x07jslink\x1b]8;;\x07 \x1b]8;;https://example.com/osc8\x07oscnk\x1b]8;;\x07 see https://example.org/auto\r\n'); await wait(200);
  const dialogs = []; page.on('dialog', (d) => { dialogs.push(d.message()); d.dismiss(); });
  const popups = []; page.context().on('page', (p) => popups.push(p.url()));
  const at = async (text) => { const b = await page.evaluate((s) => { const rows = [...document.querySelectorAll('.xterm-rows > div')]; const r = rows[0]; const i = r.textContent.indexOf(s); return i; }, text); return t.cell(g, b + 2, 1); };
  let p = await at('jslink'); await page.keyboard.down('Control'); await page.mouse.click(p.x, p.y); await page.keyboard.up('Control'); await wait(300);
  check('a javascript: link (OSC 8) does nothing, not even a prompt', popups.length === 0 && dialogs.length === 0, J({ popups, dialogs }));
  p = await at('oscnk'); await page.mouse.click(p.x, p.y); await wait(300);
  check('a plain click on a link opens nothing (it may be the start of a selection)', popups.length === 0);
  await page.keyboard.down('Control'); await page.mouse.click(p.x, p.y); await page.keyboard.up('Control'); await wait(500);
  check('Ctrl+click on an https link opens it in a new tab', popups.some((u) => u.startsWith('https://example.com/osc8')), J(popups));
  p = await at('https://example.org/auto'); await page.keyboard.down('Control'); await page.mouse.click(p.x, p.y); await page.keyboard.up('Control'); await wait(500);
  check('Ctrl+click on a plain address in the output opens it', popups.some((u) => u.startsWith('https://example.org/auto')), J(popups));
  await emit('\x07'); await wait(50);
  check('the bell rings the terminal visibly', (await page.locator('[role=region].terminal[data-bell]').count()) === 1);

  // Find.
  await emit('\x1b[2J\x1b[H' + lines(200, (i) => `needle-${i % 50 === 0 ? 'X' : 'y'} ${i}`)); await wait(300);
  await page.mouse.click(g.x + 50, g.y + 50);
  await page.keyboard.press('Control+Shift+F'); await wait(100);
  check('Ctrl+Shift+F opens find', (await page.locator('.terminal-find').count()) === 1);
  await page.keyboard.type('needle-X'); await wait(200);
  if (shots) await page.screenshot({ path: join(shots, 'find.png') });
  const shown = await page.evaluate(() => [...document.querySelectorAll('.xterm-rows > div')].some((r) => r.textContent.includes('needle-X')));
  check('find scrolls a match into view', shown && (await page.locator('.terminal-find [role=status]').count()) === 0);
  await page.keyboard.press('Escape'); await wait(100);
  check('Esc closes find', (await page.locator('.terminal-find').count()) === 0);
  await t.ctx.close();
}

// ---- 3. Rendering ----
{
  const t = await open();
  const { page, emit } = t;
  const g = await t.geo();
  await emit('\x1b[2J\x1b[HA\u{1F600}B|\r\n中文|\r\n┌──┐\r\n│██│\r\n│▓▒│\r\n└──┘\r\n');
  await wait(300);
  const widths = await page.evaluate(() => [...document.querySelectorAll('.xterm-rows > div')].slice(0, 2).map((r) => [...r.children].map((s) => [s.textContent, s.getBoundingClientRect().width])));
  const emoji = widths[0].find(([s]) => s === '\u{1F600}');
  check('an emoji takes two cells, as on the node screen', emoji && Math.abs(emoji[1] - 2 * g.cw) < 1, J(widths[0]));
  // Full blocks must be solid: the share of a block of 40 x 6 full-block characters that is not filled (holes between rows and seams).
  const { PNG } = createRequire(join(here, '../../apps/web/package.json'))('pngjs');
  const holes = async (prefs) => {
    const u = await open({ prefs });
    await u.emit('\x1b[2J\x1b[H' + ('\u2588'.repeat(40) + '\r\n').repeat(6)); await wait(500);
    const g2 = await u.geo();
    const png = PNG.sync.read(await u.page.screenshot({ clip: { x: g2.x, y: g2.y, width: g2.cw * 40, height: g2.ch * 6 } }));
    let bad = 0;
    for (let i = 0; i < png.data.length; i += 4) if (png.data[i] < 200) bad++;
    if (shots) await u.page.screenshot({ path: join(shots, `blocks-${prefs.gpu ? 'gpu' : 'page'}-${prefs.lineHeight}.png`), clip: { x: g2.x, y: g2.y, width: 330, height: g2.ch * 7 } });
    await u.ctx.close();
    return (100 * bad) / (png.width * png.height);
  };
  const gl = await holes({ gpu: true, lineHeight: 1 });
  const dom1 = await holes({ gpu: false, lineHeight: 1 });
  const dom11 = await holes({ gpu: false, lineHeight: 1.1 });
  check('full blocks are solid with the graphics card drawing (the default)', gl === 0, `${gl.toFixed(2)}% holes`);
  check('the page\'s own text drawing leaves holes in them, more at row height 1.1 (why the card draws and 1.0 is the default)', dom1 > gl && dom11 > dom1, `${dom1.toFixed(2)}% at 1.0, ${dom11.toFixed(2)}% at 1.1`);
  // Fit: the columns use the window width (less the scrollbar's strip and the padding).
  const free = await page.evaluate(() => { const el = document.querySelector('.terminal'); const cs = getComputedStyle(el); return el.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight); });
  check('fit: the unused width is under two cells and the scrollbar strip', free - g.w < 2 * g.cw + 16, `${(free - g.w).toFixed(1)}px left of ${free}`);
  // Flicker: replacing the screen with a snapshot never paints an empty screen.
  await emit(lines(30, (i) => `stable ${i}`)); await wait(200);
  await page.evaluate(() => { window.__blank = 0; window.__frames = 0; const tick = () => { window.__frames++; if (!document.querySelector('.xterm-rows > div')?.textContent.trim()) window.__blank++; if (!window.__stop) requestAnimationFrame(tick); }; requestAnimationFrame(tick); });
  const snap = '\x1b[0m\x1b[2J' + Array.from({ length: 28 }, (_, i) => `\x1b[${i + 1};1Hsnapshot row ${i}`).join('') + '\x1b[?25h';
  for (let i = 0; i < 20; i++) { await emit(snap, 'snapshot'); await wait(25); }
  const fl = await page.evaluate(() => { window.__stop = true; return [window.__blank, window.__frames]; });
  check('a snapshot (reconnect) never paints an empty screen between clear and repaint', fl[0] === 0, `${fl[0]} blank of ${fl[1]} frames`);
  await t.ctx.close();
}

// ---- 4. Settings change what is drawn ----
{
  const t = await open();
  const { page, emit } = t;
  await emit('\x1b[2J\x1b[H\x1b[38;2;30;30;70mdark blue text\x1b[0m');
  const colour = () => page.evaluate(() => { const s = [...document.querySelectorAll('.xterm-rows span')].find((x) => x.textContent.includes('dark blue')); return s ? getComputedStyle(s).color : ''; });
  await wait(150);
  const off = await colour();
  await page.evaluate(() => { localStorage.setItem('agora.terminal.prefs', JSON.stringify({ lineHeight: 1.1, readable: true, scrollback: 1000, copyOnSelect: true, gpu: false })); window.dispatchEvent(new Event('agora-terminal-prefs')); });
  // the lab's page has no settings menu; the preference record is what the menu writes, and a new terminal reads it
  await page.reload(); await page.waitForSelector('.xterm-screen'); await emit('\x1b[2J\x1b[H\x1b[38;2;30;30;70mdark blue text\x1b[0m'); await wait(200);
  const on = await colour();
  const rowH = await page.evaluate(() => document.querySelector('.xterm-rows > div').getBoundingClientRect().height);
  check('readable colours off draws the program\'s own blue, on lifts it', off !== on && off === 'rgb(30, 30, 70)', `${off} / ${on}`);
  check('row height 1.1 is a setting', rowH > 14, `${rowH}`);
  await emit(lines(30)); await wait(100);
  await page.mouse.dblclick(40, 30); await wait(150);
  check('copy on select puts the selection on the clipboard by itself', (await t.clip()).length > 0, J(await t.clip()));
  await t.ctx.close();
}

// ---- 5. Smaller terminal in a bigger window; touch; both themes ----
for (const scheme of ['light', 'dark']) {
  const t = await open({ scheme });
  const { page, emit } = t;
  await page.evaluate(() => { window.__noecho = true; });
  await page.evaluate(() => window.__resize(80, 24)); await wait(300);
  await emit('\x1b[2J\x1b[Hsmaller than the window\r\n' + '+' + '-'.repeat(78) + '+');
  await wait(200);
  if (shots) await page.screenshot({ path: join(shots, `letterbox-${scheme}.png`) });
  await t.ctx.close();
}
for (const scheme of ['dark', 'light']) {
  const t = await open({ width: 390, height: 760, touch: true, dpr: 2, scheme });
  const { page, emit } = t;
  await emit(lines(300, (i) => `swipe line ${i}`)); await wait(300);
  const before = await t.top();
  const cdp = await t.ctx.newCDPSession(page);
  const box = await page.locator('.xterm-screen').boundingBox();
  const x = Math.round(box.x + 100), y0 = Math.round(box.y + 100);
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y: y0 }] });
  for (let i = 1; i <= 8; i++) { await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x, y: y0 + i * 30 }] }); await wait(16); }
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await wait(500);
  check('phone: a swipe down scrolls the history up', (await t.top()) !== before, `${before} -> ${await t.top()}`);
  if (shots) await page.screenshot({ path: join(shots, `phone-${scheme}.png`) });
  await t.ctx.close();
}
await browser.close();
console.log(failed ? `\n${failed} check(s) failed` : '\nall checks passed');
process.exit(failed ? 1 : 0);
