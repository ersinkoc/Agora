// Real-browser check for Home without stripes (ADR-0102, redesign PR 2): on the built demo window (#/demo), every agent tile, every request
// card of what needs you and every count, with everything inside them and their ::before and ::after, has no bar on one side: a border is
// the same width on all four sides or absent, and no inset shadow is offset to one side. A tile never glows or pulses (its animation is
// none, with motion allowed); the needs-you pulse is the state mark's only. Desktop and phone, dark and light.
//
//   cd apps/web && npm ci && npm run build
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-home-no-stripes.mjs
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'apps/web/dist');
const { chromium } = createRequire(join(process.env.PW_DIR ?? '.', 'x.js'))('playwright-core');
const exe = process.env.CHROME ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome`;
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm' };

const server = createServer((req, res) => {
  const path = join(dist, !req.url || req.url === '/' ? 'index.html' : req.url.split('?')[0]);
  if (!existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
}).listen(0, '127.0.0.1');
await new Promise((r) => server.once('listening', r));
const base = `http://127.0.0.1:${server.address().port}/#/demo`;

// What is checked, in the page: each root and every element under it, with its two pseudo-elements.
function inspect() {
  const roots = [...document.querySelectorAll('.fleet-console .tile, .fleet-console .fleet-ask, .fleet-summary .kit-stat')];
  const problems = [];
  const px = (v) => Number.parseFloat(v) || 0;
  const name = (el) => `${el.tagName.toLowerCase()}.${[...el.classList].join('.')}`;
  for (const r of roots) {
    for (const el of [r, ...r.querySelectorAll('*')]) {
      for (const pseudo of [null, '::before', '::after']) {
        const s = getComputedStyle(el, pseudo);
        if (pseudo && (s.content === 'none' || s.content === 'normal')) continue;
        const w = ['Top', 'Right', 'Bottom', 'Left'].map((side) => (s[`border${side}Style`] === 'none' ? 0 : px(s[`border${side}Width`])));
        if (Math.max(...w) > 0 && new Set(w).size > 1) problems.push(`${name(el)}${pseudo ?? ''}: border ${w.join('/')}`);
        // Each inset shadow: "rgb(...) X Y blur spread inset". A ring (0 0 0 n) is fine; any offset is a bar on one side.
        for (const part of s.boxShadow === 'none' ? [] : s.boxShadow.split(/,(?![^(]*\))/)) {
          if (!part.includes('inset')) continue;
          const [x, y] = (part.replace(/rgba?\([^)]*\)|color\([^)]*\)|oklab\([^)]*\)/g, '').match(/-?[\d.]+px/g) ?? []).map(px);
          if (x || y) problems.push(`${name(el)}${pseudo ?? ''}: inset shadow ${part.trim()}`);
        }
      }
    }
    if (r.classList.contains('tile')) {
      const s = getComputedStyle(r);
      if (s.animationName !== 'none') problems.push(`${name(r)}: animation ${s.animationName}`);
      if (s.boxShadow !== 'none') problems.push(`${name(r)}: shadow ${s.boxShadow}`);
    }
  }
  return { count: { tiles: roots.filter((r) => r.classList.contains('tile')).length, cards: roots.filter((r) => r.classList.contains('fleet-ask')).length, stats: roots.filter((r) => r.classList.contains('kit-stat')).length }, problems };
}

const browser = await chromium.launch({ executablePath: exe, args: ['--no-sandbox'] });
let failed = 0;
for (const [vname, viewport, phone] of [['desktop', { width: 1440, height: 900 }, false], ['phone', { width: 390, height: 844 }, true]]) {
  for (const scheme of ['dark', 'light']) {
    const ctx = await browser.newContext({ viewport, colorScheme: scheme, reducedMotion: 'no-preference', ...(phone ? { isMobile: true, hasTouch: true } : {}) });
    await ctx.addInitScript(() => { try { localStorage.setItem('agora.attention.mode', 'off'); } catch { /* the check runs without it */ } });
    const page = await ctx.newPage();
    await page.goto(base);
    await page.waitForSelector('.fleet-summary');
    await page.waitForTimeout(800);
    const { count, problems } = await page.evaluate(inspect);
    // The desktop demo has tiles and a request card; the phone draws its own list (redesign PR 13), so only its counts are checked there.
    const enough = phone ? count.stats >= 5 : count.tiles >= 4 && count.cards >= 1 && count.stats >= 5;
    if (!enough || problems.length) { failed++; console.log(`FAIL ${vname} ${scheme} ${JSON.stringify(count)}`); for (const p of problems) console.log(`  ${p}`); }
    else console.log(`ok   ${vname} ${scheme}: ${count.tiles} tiles, ${count.cards} request cards, ${count.stats} counts, no bar on one side`);
    await ctx.close();
  }
}
await browser.close();
server.close();
process.exit(failed ? 1 : 0);
