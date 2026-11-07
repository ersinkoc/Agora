# Web client

Primary client (ADR-0005). Framework: ADR-0017. Built assets are embedded into the hub binary.

```
npm ci
npm run dev        # Vite dev server; #/gallery shows every component in both themes
npm run typecheck && npm test && npm run build
npm run gen        # TypeScript types from schema/ into src/gen (not committed)
```

- `src/design/tokens.css`: design tokens (colors for both themes, type scale, spacing, radius, motion). Components use
  only these variables.
- `src/components/`: pure components that render a view model (`src/model.ts`); `src/screens.tsx`: Home and the gallery.
- Text comes only from `locales/en/web.ftl` through `src/i18n.ts`. A test fails when a key is missing.
- `src/terminal/`: the renderer interface (xterm.js behind it, lazy-loaded), terminal frame decoding, and a demo source.
- The demo data in `src/fixtures.ts` stands in until the hub connection feeds the store.
