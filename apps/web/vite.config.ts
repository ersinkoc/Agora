import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

// `locales/` lives at the repository root and is shared with the Rust code (ADR-0007, ADR-0017). Tailwind is build-time only (ADR-0036).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: { fs: { allow: ['../..'] } },
  test: { environment: 'node' },
});
