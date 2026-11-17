// Serves the lab page at http://127.0.0.1:5199/lab (LAB_PORT), through apps/web's own Vite setup so Tailwind finds the same classes the
// app uses: `cd apps/web && npx vite --config ../../scripts/terminal-fidelity/vite.config.mjs`.
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
const here = dirname(fileURLToPath(import.meta.url));
const web = resolve(here, '../../apps/web');
const req = createRequire(`${web}/package.json`);
const { defineConfig } = req('vite');
const react = req('@vitejs/plugin-react');
const tailwind = req('@tailwindcss/vite');
const lab = {
  name: 'terminal-fidelity-lab',
  configureServer(server) {
    server.middlewares.use('/lab', async (_req, res) => {
      const html = readFileSync(`${here}/index.html`, 'utf8').replace('./lab.tsx', `/@fs${here}/lab.tsx`);
      res.setHeader('content-type', 'text/html');
      res.end(await server.transformIndexHtml('/lab', html));
    });
  },
};
export default defineConfig({
  root: web,
  plugins: [(react.default ?? react)(), (tailwind.default ?? tailwind)(), lab],
  server: { port: Number(process.env.LAB_PORT || 5199), host: '127.0.0.1', strictPort: true, fs: { allow: [resolve(here, '../..')] } },
});
