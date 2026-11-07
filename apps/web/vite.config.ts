import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// `locales/` lives at the repository root and is shared with the Rust code (ADR-0007, ADR-0017).
export default defineConfig({
  plugins: [react()],
  server: { fs: { allow: ['../..'] } },
  test: { environment: 'node' },
});
