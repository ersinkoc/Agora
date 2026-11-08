import { chromium } from 'playwright-core';
const exe = process.env.CHROME ?? process.env.HOME + '/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome';
const b = await chromium.launch({ executablePath: exe, args: ['--no-sandbox', '--enable-precise-memory-info', '--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
for (const engine of ['xterm', 'xterm-webgl', 'ghostty']) {
  const page = await (await b.newContext()).newPage();
  page.on('pageerror', (e) => console.log(engine, 'PAGEERROR', e.message));
  await page.goto(`http://127.0.0.1:5199/?engine=${engine}&mb=${process.argv[2] || 20}`);
  await page.waitForFunction(() => window.__result, null, { timeout: 300000 });
  const r = await page.evaluate(() => window.__result);
  const f = (x) => `${x.mbps.toFixed(1)} MB/s, ${x.secs.toFixed(1)} s, long tasks ${x.longTasks} (longest ${x.longest.toFixed(0)} ms)`;
  console.log(`${engine.padEnd(12)} plain: ${f(r.plain)}\n${''.padEnd(12)} color: ${f(r.color)}\n${''.padEnd(12)} echo write p50 ${r.latency.writeP50.toFixed(1)} p95 ${r.latency.writeP95.toFixed(1)} ms; next frame p50 ${r.latency.frameP50.toFixed(1)} p95 ${r.latency.frameP95.toFixed(1)} ms; heap ${r.heapMB.toFixed(0)} MB`);
  await page.close();
}
await b.close();
