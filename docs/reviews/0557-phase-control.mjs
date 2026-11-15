// Native-browser regression control for #557. Instrument the current benchmark, not a copied implementation.
// Run with the benchmark's PW_DIR / AGORA_PERF_BIN / AGORA_PERF_WEB_DIR environment.
import { readFileSync, writeFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const root = new URL('../../', import.meta.url);
const source = new URL('scripts/perf-tree.mjs', root);
const generated = new URL(`scripts/.phase-control-${process.pid}.mjs`, root);
function replaceOnce(text, marker, replacement) {
  if (text.split(marker).length !== 2) throw new Error(`Benchmark control marker missing or ambiguous: ${marker}`);
  return text.replace(marker, replacement);
}
let code = readFileSync(source, 'utf8');
code = replaceOnce(code, '    phases.scroll.end = performance.now();', `    phases.scroll.end = performance.now();
    await new Promise((r) => setTimeout(r, 0));
    const controlStart = performance.now();
    while (performance.now() - controlStart < 80) { /* controlled post-scroll task */ }
    const controlEnd = performance.now();
    await new Promise((r) => setTimeout(r, 0));`);
code = replaceOnce(code, 'return { ...size, scrollFrameMs:', `return { ...size,
      reviewControl: { controlStart, controlEnd, phases, entries: long.map((e) => ({ start: e.startTime, duration: e.duration })) },
      scrollFrameMs:`);
code = replaceOnce(code, '  console.log(JSON.stringify(', `  const c = result.reviewControl;
  const controlled = c.entries.filter((e) => e.start <= c.controlStart && e.start + e.duration >= c.controlEnd - 1);
  if (controlled.length !== 1) throw new Error('Controlled post-scroll task not observed exactly once');
  if (!Object.values(c.phases).every((p) => controlled[0].start < p.start || controlled[0].start >= p.end)) throw new Error('Control entered a measured phase');
  if (result.longTasksOutsidePhases < 1) throw new Error('Control not counted outside phases');
  const scroll = c.entries.filter((e) => e.start >= c.phases.scroll.start && e.start < c.phases.scroll.end);
  if (result.longTasksWhileScrolling !== scroll.length) throw new Error('Scroll counter includes another phase');
  const sum = result.longTasksWhileScrolling + result.longTasksWhilePicking + result.longTasksWhileFiltering + result.longTasksOutsidePhases;
  if (sum !== result.longTasksTotal) throw new Error('Phase totals disagree');
  result.reviewControlPassed = true;
  console.log(JSON.stringify(`);
try {
  writeFileSync(generated, code);
  const result = spawnSync(process.execPath, [fileURLToPath(generated)], { stdio: 'inherit', env: process.env });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
} finally {
  rmSync(generated, { force: true });
}
