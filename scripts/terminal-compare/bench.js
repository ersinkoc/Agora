import { Terminal as Xterm } from '@xterm/xterm';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { init as ghosttyInit, Terminal as Ghostty } from 'ghostty-web';

const engine = new URLSearchParams(location.search).get('engine');
const MB = Number(new URLSearchParams(location.search).get('mb') || 20);
const el = document.getElementById('t');
let term;
if (engine === 'ghostty') { await ghosttyInit(); term = new Ghostty({ fontSize: 13, cols: 100, rows: 30 }); term.open(el); }
else { term = new Xterm({ fontSize: 13, cols: 100, rows: 30, scrollback: 2000 }); term.open(el); if (engine === 'xterm-webgl') term.loadAddon(new WebglAddon()); }
await new Promise((r) => setTimeout(r, 500));

const enc = new TextEncoder();
const plainLine = '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n';
const colorLine = '\x1b[32mgreen\x1b[0m \x1b[1;31mred bold\x1b[0m \x1b[38;5;208morange\x1b[0m 0123456789abcdef0123456789abcdef\r\n';
const chunkOf = (line) => enc.encode(line.repeat(Math.floor(16384 / line.length)));
const long = [];
new PerformanceObserver((l) => l.getEntries().forEach((e) => long.push(e.duration))).observe({ entryTypes: ['longtask'] });

async function flood(line, mb) {
  const chunk = chunkOf(line);
  const n = Math.ceil((mb * 1e6) / chunk.length);
  long.length = 0;
  const t0 = performance.now();
  await new Promise((resolve) => {
    let sent = 0, done = 0;
    const pump = () => { // keep up to 64 chunks in flight, as a socket would
      while (sent < n && sent - done < 64) { sent++; term.write(chunk, () => { done++; if (done === n) resolve(); else pump(); }); }
    };
    pump();
  });
  await new Promise((r) => requestAnimationFrame(() => r()));
  const secs = (performance.now() - t0) / 1000;
  return { mbps: mb / secs, secs, longTasks: long.length, longest: long.length ? Math.max(...long) : 0 };
}

async function latency(n) {
  const out = [];
  for (let i = 0; i < n; i++) {
    const t0 = performance.now();
    await new Promise((resolve) => term.write('a', resolve));
    const written = performance.now() - t0;
    await new Promise((r) => requestAnimationFrame(() => r()));
    out.push([written, performance.now() - t0]);
    await new Promise((r) => setTimeout(r, 20));
  }
  const p = (xs, q) => xs.slice().sort((a, b) => a - b)[Math.floor(xs.length * q)];
  return { writeP50: p(out.map((x) => x[0]), 0.5), writeP95: p(out.map((x) => x[0]), 0.95), frameP50: p(out.map((x) => x[1]), 0.5), frameP95: p(out.map((x) => x[1]), 0.95) };
}

const result = { engine };
result.plain = await flood(plainLine, MB);
result.color = await flood(colorLine, MB);
result.latency = await latency(100);
result.heapMB = performance.memory ? performance.memory.usedJSHeapSize / 1e6 : -1;
window.__result = result;
