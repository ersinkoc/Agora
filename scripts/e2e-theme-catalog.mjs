// ADR-0113 decision 6 in a real browser on a real hub: the owner fetches a catalog served on this machine (`agora hub themes update`)
// while the hub runs, the window shows nothing until the owner turns it on, then lists it; a person previews and installs a theme,
// and every request the page makes stays on the hub's own origin. Uses a disposable hub and node, never a real account or catalog.
//   cargo build -p agora -p agora-fake-agent && (cd apps/web && npm run build)
//   PW_DIR=<dir with node_modules/playwright-core> node scripts/e2e-theme-catalog.mjs
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { createServer } from 'node:http';
import { promisify } from 'node:util';
import { startStack } from './browser-harness.mjs';

const HARBOUR = JSON.stringify({ 'agora-theme': 2, name: 'Harbour', inherits: 'slate', dark: { tokens: { accent: '#5cc8c2' } }, light: { tokens: { accent: '#0b6e69', 'on-accent': '#ffffff' } } });
const BAD = JSON.stringify({ 'agora-theme': 1, name: 'Bad', scheme: 'dark', tokens: {}, script: 'alert(1)' });
const sha = (s) => createHash('sha256').update(s).digest('hex');
const entry = (id, name, data) => ({ id, name, version: 1, author: 'Test', licence: 'MIT', sha256: sha(data), file: `themes/${id}.json` });
const files = {
  '/cat/index.json': JSON.stringify({ 'agora-theme-catalog': 1, themes: [entry('harbour', 'Harbour', HARBOUR), entry('bad', 'Bad', BAD)] }),
  '/cat/themes/harbour.json': HARBOUR,
  '/cat/themes/bad.json': BAD,
};
const catalog = createServer((req, res) => {
  const body = files[req.url ?? ''];
  if (body === undefined) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': 'application/json' }).end(body);
}).listen(0, '127.0.0.1');
await new Promise((r) => catalog.once('listening', r));
const catalogUrl = `http://127.0.0.1:${catalog.address().port}/cat/index.json`;

let stack;
try {
  stack = await startStack({ binary: process.env.AGORA_TEST_BINARY, ...(process.env.AGORA_TEST_EMBEDDED_WEB ? { web: '' } : {}) });
  const { page, run, hubDir, origin, errors, bin, env } = stack;
  // Not the harness's `run`: that blocks this process, which also serves the catalog.
  const runAsync = (args) => promisify(execFile)(bin, args, { env }).then((r) => `${r.stdout}${r.stderr}`, (e) => `${e.stdout}${e.stderr}`);
  page.setDefaultTimeout(15000);
  const elsewhere = [];
  page.on('request', (r) => { const u = new URL(r.url()); if (u.origin !== new URL(origin).origin && !['data:', 'blob:'].includes(u.protocol)) elsewhere.push(r.url()); });
  const openEditor = async () => {
    await page.goto(`${origin}/#/`);
    await page.waitForSelector('.ide');
    await page.keyboard.press('Control+k');
    await page.keyboard.type('Edit theme');
    await page.keyboard.press('Enter');
    await page.waitForSelector('section[aria-label="Theme"]');
  };

  // The owner fetches while the hub runs; the bad theme is refused and named, and the command says so.
  const out = await runAsync(['hub', 'themes', 'update', '--url', catalogUrl, '--hub-dir', hubDir]);
  assert.match(out, /1 themes kept, 1 refused/, out);
  assert.match(out, /Refused bad/, out);

  // Off: the editor shows no catalog.
  await openEditor();
  await page.waitForTimeout(1500);
  assert.equal(await page.locator('[data-theme-catalog]').count(), 0, 'no catalog while it is off');

  // On: the running hub follows the switch without a restart. The editor asks when it opens, so the page is loaded again.
  assert.match(run(['hub', 'themes', 'on', '--hub-dir', hubDir]), /catalog is on/);
  await page.reload();
  await openEditor();
  const harbour = page.locator('[data-catalog-entry="harbour"]');
  await harbour.waitFor();
  assert.equal(await page.locator('[data-catalog-entry="bad"]').count(), 0, 'a refused theme is never offered');
  await harbour.getByRole('button', { name: 'Preview' }).click();
  await harbour.locator('[data-catalog-preview]').waitFor();
  await harbour.getByRole('button', { name: 'Install' }).click();
  await harbour.getByText('Installed', { exact: true }).waitFor();
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('agora.themes.v1') ?? '[]'));
  assert.deepEqual(stored.map((s) => s.catalog), [{ id: 'harbour', version: 1 }]);
  assert.equal(stored[0].file.name, 'Harbour');

  assert.deepEqual(elsewhere, [], 'the window asked no other origin');
  assert.deepEqual(errors, []);
  console.log('ok: the hub fetched the catalog, the window showed it only once on, previewed and installed a theme, same origin only');
} finally {
  await stack?.close();
  catalog.close();
}
