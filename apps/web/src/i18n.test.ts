import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { t } from './i18n';

// Every key the UI asks for must exist in locales/en/web.ftl (a missing key would render as its id).
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? (e.name === 'gen' ? [] : sources(`${dir}/${e.name}`)) : /\.tsx?$/.test(e.name) && !/\.test\./.test(e.name) ? [`${dir}/${e.name}`] : []);
}

describe('i18n', () => {
  it('has every literal key used with t()', () => {
    const keys = new Set<string>();
    for (const f of sources('src')) {
      for (const m of readFileSync(f, 'utf8').matchAll(/\bt\('([a-z0-9-]+)'/g)) keys.add(m[1] as string);
    }
    expect(keys.size).toBeGreaterThan(5);
    for (const k of keys) expect(t(k, { machine: 'm', name: 'n', count: 1 }), k).not.toBe(k);
  });
  it('has the dynamic state and theme keys', () => {
    for (const s of ['working', 'needs-you', 'finished', 'idle', 'unknown']) expect(t(`state-${s}`)).not.toBe(`state-${s}`);
    for (const s of ['dark', 'light']) expect(t(`theme-${s}`)).not.toBe(`theme-${s}`);
  });
  it('pluralizes', () => {
    expect(t('machines-online', { count: 1 })).toBe('1 machine online');
    expect(t('machines-online', { count: 2 })).toBe('2 machines online');
  });
});
