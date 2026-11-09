import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { directionOf, pickLanguage, setLanguage, t } from './i18n';

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
    for (const k of keys) expect(t(k, { machine: 'm', name: 'n', count: 1, amount: '$1', date: 'd' }), k).not.toBe(k);
  });
  it('has the dynamic state and theme keys', () => {
    for (const s of ['working', 'needs-you', 'finished', 'idle', 'unknown']) expect(t(`state-${s}`)).not.toBe(`state-${s}`);
    for (const s of ['dark', 'light']) expect(t(`theme-${s}`)).not.toBe(`theme-${s}`);
  });
  it('lays the page out right to left for the languages written that way', () => {
    for (const l of ['ar', 'he', 'fa-IR', 'ur']) expect(directionOf(l), l).toBe('rtl');
    for (const l of ['en', 'tr', 'de', '']) expect(directionOf(l), l).toBe('ltr');
  });
  it('pluralizes', () => {
    expect(t('machines-online', { count: 1 })).toBe('1 machine online');
    expect(t('machines-online', { count: 2 })).toBe('2 machines online');
  });
  it('speaks Turkish when asked and falls back to English per message', () => {
    expect(pickLanguage('tr-TR')).toBe('tr');
    expect(pickLanguage('de')).toBe('en');
    expect(pickLanguage(undefined)).toBe('en');
    setLanguage('tr');
    expect(t('state-needs-you')).toBe('Sizi bekliyor');
    expect(t('machines-online', { count: 2 })).toBe('2 makine çevrimiçi');
    setLanguage('en');
    expect(t('state-needs-you')).toBe('Needs you');
  });
  it('has a Turkish message for every English one', () => {
    const ids = (src: string) => [...src.matchAll(/^([a-z0-9-]+) =/gm)].map((m) => m[1]);
    const en = ids(readFileSync('../../locales/en/web.ftl', 'utf8'));
    const tr = new Set(ids(readFileSync('../../locales/tr/web.ftl', 'utf8')));
    expect(en.filter((k) => !tr.has(k as string))).toEqual([]);
  });
});
