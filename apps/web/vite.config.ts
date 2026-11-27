import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { readFileSync } from 'node:fs';

// The release this window is part of (the workspace's version), so a plugin package can say the oldest Agora it runs in (ADR-0114).
const version = /^version = "([^"]+)"/m.exec(readFileSync(new URL('../../Cargo.toml', import.meta.url), 'utf8'))?.[1] ?? '0.0.0';

// `locales/` lives at the repository root and is shared with the Rust code (ADR-0007, ADR-0017). Tailwind is build-time only (ADR-0036).
// `long` (through protobufjs) first tries a tiny WebAssembly module for 64-bit arithmetic; the hub's page policy refuses WebAssembly on
// purpose (no 'wasm-unsafe-eval'), which printed a CSP error at every load before `long` fell back to plain JavaScript. The probe is
// taken out of the bundle, so `long` uses plain JavaScript from the start; the page policy stays as it is.
const longWithoutWasm = {
  name: 'long-without-wasm',
  enforce: 'pre' as const,
  transform(code: string, id: string) {
    if (!/[\\/]node_modules[\\/]long[\\/]/.test(id) || !code.includes('new WebAssembly.Instance')) return null;
    const out = code.replace(/try\s*\{\s*wasm\s*=\s*new WebAssembly\.Instance\([\s\S]*?\}\s*catch\s*(\(\w+\)\s*)?\{[^}]*\}/, '/* WebAssembly probe removed: the page policy refuses it */');
    if (out === code) throw new Error('long-without-wasm: the WebAssembly probe was not found; update the pattern');
    return { code: out, map: null };
  },
};

export default defineConfig({
  plugins: [longWithoutWasm, react(), tailwindcss()],
  define: { __AGORA_VERSION__: JSON.stringify(version) },
  server: { fs: { allow: ['../..'] } },
  test: { environment: 'node' },
});
