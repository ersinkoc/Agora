import { afterEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { PRESETS } from './engine';
import { PANELS } from './tree';
import { panelLabel } from './Workspace';

// The layout's keys built at run time (the key test in src/i18n.test.ts sees only literal ones), in both languages.
describe('layout messages', () => {
  afterEach(() => setLanguage('en'));
  for (const lang of ['en', 'tr']) {
    it(`has every preset, error, follow mode and panel name in ${lang}`, () => {
      setLanguage(lang);
      const keys = [
        ...PRESETS.map((p) => `layout-preset-${p}`),
        ...['json', 'invalid', 'newer', 'too-deep', 'too-many'].map((e) => `layout-error-${e}`),
        ...['attention', 'selection'].flatMap((f) => [`layout-follow-${f}`, `layout-follows-${f}`]),
      ];
      for (const k of keys) expect(t(k, { max: 1 }), k).not.toBe(k);
      for (const p of PANELS) expect(panelLabel(p), p).not.toMatch(/^[a-z-]+$/);
      expect(panelLabel('machines-map')).toContain('machines-map');
    });
  }
});
