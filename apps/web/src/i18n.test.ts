import { readFileSync, readdirSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { actionKey } from './net/audit';
import { chooseLanguage, directionOf, getLanguage, getLanguagePreference, pickLanguage, setLanguage, subscribeLanguage, t } from './i18n';

// Every key the UI asks for must exist in locales/en/web.ftl (a missing key would render as its id).
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? (e.name === 'gen' ? [] : sources(`${dir}/${e.name}`)) : /\.tsx?$/.test(e.name) && !/\.test\./.test(e.name) ? [`${dir}/${e.name}`] : []);
}

describe('i18n', () => {
  // Node exposes the system locale through navigator too; translation assertions must choose their own language.
  beforeEach(() => setLanguage('en'));
  it('has every literal key used with t()', () => {
    const keys = new Set<string>();
    for (const f of sources('src')) {
      for (const m of readFileSync(f, 'utf8').matchAll(/\bt\('([a-z0-9-]+)'/g)) keys.add(m[1] as string);
    }
    expect(keys.size).toBeGreaterThan(5);
    for (const k of keys) expect(t(k, { machine: 'm', id: 'i', name: 'n', count: 1, online: 1, total: 1, when: 'w', code: 1, reason: 'r', amount: '$1', date: 'd', percent: 1, used: 'u', size: 's', stderr_tail: 't' }), k).not.toBe(k);
  });
  it('has the dynamic state and theme keys', () => {
    for (const s of ['working', 'needs-you', 'finished', 'idle', 'unknown']) expect(t(`state-${s}`)).not.toBe(`state-${s}`);
    for (const s of ['dark', 'light']) expect(t(`theme-${s}`)).not.toBe(`theme-${s}`);
  });
  it('has the audit trail\'s reasons and outcomes', () => {
    for (const k of ['owner-only', 'signed-out', 'failed']) expect(t(`audit-${k}`)).not.toBe(`audit-${k}`);
    for (const k of ['done', 'sent']) expect(t(`audit-outcome-${k}`)).not.toBe(`audit-outcome-${k}`);
    for (const a of ['sign_in', 'user.add', 'AgentStop', 'ApprovalResolve', 'node.resynced']) expect(t(actionKey(a)), a).not.toBe(actionKey(a));
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

describe('browser language preference', () => {
  afterEach(() => {
    chooseLanguage('system');
    setLanguage('en');
    vi.unstubAllGlobals();
  });

  it('uses an explicit choice until the user returns to the browser language', () => {
    const saved = new Map<string, string>();
    vi.stubGlobal('navigator', { language: 'tr-TR' });
    vi.stubGlobal('localStorage', {
      setItem: (key: string, value: string) => saved.set(key, value),
      removeItem: (key: string) => saved.delete(key),
    });
    expect(chooseLanguage('en')).toBe(true);
    expect(getLanguage()).toBe('en');
    expect(getLanguagePreference()).toBe('en');
    expect(saved.get('ui.language')).toBe('en');
    expect(chooseLanguage('system')).toBe(true);
    expect(saved.has('ui.language')).toBe(false);
    expect(getLanguagePreference()).toBe('system');
    expect(getLanguage()).toBe('tr');
  });

  it('updates the current page even when persistence is refused', () => {
    vi.stubGlobal('localStorage', {
      setItem: () => { throw new Error('storage refused'); },
      removeItem: () => { throw new Error('storage refused'); },
    });
    const documentElement = { lang: '', dir: '' };
    vi.stubGlobal('document', { documentElement });
    const listener = vi.fn();
    const unsubscribe = subscribeLanguage(listener);
    expect(chooseLanguage('tr')).toBe(false);
    expect(t('language-label')).toBe('Dil');
    expect(documentElement).toEqual({ lang: 'tr', dir: 'ltr' });
    expect(listener).toHaveBeenCalledOnce();
    unsubscribe();
    chooseLanguage('en');
    expect(listener).toHaveBeenCalledOnce();
  });
});
