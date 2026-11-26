import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { readFileSync } from 'node:fs';

// The release this window is part of (the workspace's version), so a plugin package can say the oldest Agora it runs in (ADR-0114).
const version = /^version = "([^"]+)"/m.exec(readFileSync(new URL('../../Cargo.toml', import.meta.url), 'utf8'))?.[1] ?? '0.0.0';

// `locales/` lives at the repository root and is shared with the Rust code (ADR-0007, ADR-0017). Tailwind is build-time only (ADR-0036).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: { __AGORA_VERSION__: JSON.stringify(version) },
  server: { fs: { allow: ['../..'] } },
  test: { environment: 'node' },
});
