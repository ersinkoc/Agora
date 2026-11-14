import { describe, expect, it } from 'vitest';
import { CHOICE_KEY, MAX_USER_THEMES, MODE_KEY, USER_KEY, createThemeStore, loadState } from './store';
import { BUILT_IN, baseTokens, serialiseTheme, themeFile } from './tokens';

function memory(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); }, m };
}
const broken = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('full'); }, removeItem: () => { throw new Error('denied'); } };
function page() {
  const vars = new Map<string, string>();
  return { vars, root: { style: { setProperty: (k: string, v: string) => { vars.set(k, v); }, removeProperty: (k: string) => vars.delete(k) }, dataset: {} as Record<string, string | undefined> } };
}
const sand = BUILT_IN.find((b) => b.id === 'sand')!;
const slate = BUILT_IN.find((b) => b.id === 'slate')!;

describe('the chosen theme', () => {
  it('follows the system with Light and Dark when nothing is stored, and when storage throws', () => {
    for (const kv of [memory(), broken]) {
      let light = false;
      const s = createThemeStore({ kv, prefersLight: () => light });
      expect(s.state().mode).toBe('system');
      expect(s.active().id).toBe('dark');
      light = true;
      expect(s.active().id).toBe('light');
      expect(() => { s.choose('sand'); s.setMode('dark'); s.reset(); }).not.toThrow();
    }
  });

  it('applies the theme to the page and tells the terminals', () => {
    const { vars, root } = page();
    let applied = 0;
    let light = false;
    const s = createThemeStore({ kv: memory(), root, prefersLight: () => light, onApply: () => { applied++; } });
    s.apply();
    // Dark and Light are the stylesheet's own: only the scheme is set, nothing inline.
    expect(root.dataset.theme).toBe('dark');
    expect(vars.size).toBe(0);
    light = true;
    const before = s.state();
    s.systemChanged();
    expect(s.state()).not.toBe(before); // views showing the theme's name draw again
    expect(root.dataset.theme).toBe('light');
    s.choose('sand');
    expect(vars.get('--surface-0')).toBe(sand.tokens['surface-0']);
    s.choose('light');
    expect(vars.size).toBe(0);
    s.choose('large-text');
    // Following the system (light): choosing a dark theme makes it the dark choice without forcing dark.
    expect(s.state().mode).toBe('system');
    expect(root.dataset.theme).toBe('light');
    s.setMode('dark');
    expect(vars.get('--text-scale')).toBe('1.25');
    expect(applied).toBe(6);
  });

  it('is remembered in this browser, per scheme, and a forced scheme moves to the theme chosen', () => {
    const kv = memory();
    const s = createThemeStore({ kv });
    s.setMode('light');
    s.choose('slate');
    expect(s.state().mode).toBe('dark');
    s.choose('sand');
    expect(s.state()).toMatchObject({ mode: 'light', choice: { dark: 'slate', light: 'sand' } });
    expect(kv.m.get(MODE_KEY)).toBe('light');
    expect(loadState(kv)).toEqual(s.state());
    s.reset();
    expect(kv.m.has(MODE_KEY) || kv.m.has(CHOICE_KEY)).toBe(false);
    expect(s.state()).toMatchObject({ mode: 'system', choice: { dark: 'dark', light: 'light' } });
  });

  it('reads the earlier rung\'s stored scheme, and falls back from anything it cannot use', () => {
    expect(loadState(memory({ [MODE_KEY]: 'light' })).mode).toBe('light');
    const s = loadState(memory({ [MODE_KEY]: 'sepia', [CHOICE_KEY]: JSON.stringify({ dark: 'sand', light: 'nope' }), [USER_KEY]: '{' }));
    expect(s).toEqual({ mode: 'system', choice: { dark: 'dark', light: 'light' }, user: [] });
    // A stored theme is checked like an imported one: an unreadable one is dropped, the rest kept.
    const bad = { id: 'user-2', file: themeFile({ scheme: 'dark', tokens: { ...baseTokens('dark'), text: '#111111' } }, 'Bad') };
    const good = { id: 'user-1', file: themeFile(slate, 'Good') };
    const kept = loadState(memory({ [USER_KEY]: JSON.stringify([good, bad, { id: 'x', file: good.file }, good]), [CHOICE_KEY]: JSON.stringify({ dark: 'user-2' }) }));
    expect(kept.user.map((u) => u.name)).toEqual(['Good']);
    expect(kept.choice.dark).toBe('dark');
  });
});

describe('the person\'s themes', () => {
  it('copies a built-in theme, changes it live, and refuses a change that would be hard to read', () => {
    const kv = memory();
    const { vars, root } = page();
    const s = createThemeStore({ kv, root });
    s.setMode('light');
    const c = s.copy(sand, 'Sand (mine)');
    expect(c.ok).toBe(true);
    if (!c.ok) return;
    expect(s.active().id).toBe(c.theme.id);
    expect(s.update(c.theme.id, { tokens: { accent: '#8a3a10' } }).ok).toBe(true);
    expect(vars.get('--accent')).toBe('#8a3a10');
    const r = s.update(c.theme.id, { tokens: { text: '#e0e0e0' } });
    expect(r.ok).toBe(false);
    expect(!r.ok && 'issues' in r && r.issues.every((i) => i.code === 'contrast')).toBe(true);
    expect(vars.get('--text')).toBe(sand.tokens.text); // the window stayed readable
    expect(s.update(c.theme.id, { name: '  ' }).ok).toBe(false);
    expect(s.update('dark', { tokens: { accent: '#000000' } }).ok).toBe(false); // built-in themes cannot change
    // Another tab sees it after a reload of what is stored.
    const other = createThemeStore({ kv });
    expect(other.active()).toMatchObject({ name: 'Sand (mine)', tokens: { accent: '#8a3a10' } });
  });

  it('imports a file, refuses a bad one with its reasons, exports what imports back, and deletes', () => {
    const kv = memory();
    const s = createThemeStore({ kv, prefersLight: () => true });
    const text = serialiseTheme({ 'agora-theme': 1, name: 'Night', scheme: 'dark', tokens: { accent: '#ff9f43', 'on-accent': '#1a1000' } });
    const r = s.import(text);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(s.state().choice.dark).toBe(r.theme.id);
    expect(s.active().id).toBe('light'); // following a light system: the import waits for dark
    expect(s.import('{"agora-theme":1,"name":"x","scheme":"dark","tokens":{"accent":"red"}}')).toEqual({ ok: false, issues: [{ code: 'bad-value', token: 'accent' }] });
    const again = s.import(serialiseTheme(themeFile(r.theme, r.theme.name)));
    expect(again.ok && again.theme.tokens).toEqual(r.theme.tokens);
    expect(new Set(s.state().user.map((u) => u.id)).size).toBe(2);
    s.remove(again.ok ? again.theme.id : '');
    s.remove(r.theme.id);
    expect(s.state().user).toEqual([]);
    expect(s.state().choice.dark).toBe('dark');
    expect(loadState(kv).user).toEqual([]);
  });

  it(`keeps at most ${MAX_USER_THEMES}`, () => {
    const s = createThemeStore({ kv: memory() });
    for (let i = 0; i < MAX_USER_THEMES; i++) expect(s.copy(slate, `n${i}`).ok).toBe(true);
    expect(s.copy(slate, 'one more')).toEqual({ ok: false, full: true });
    expect(new Set(s.state().user.map((u) => u.id)).size).toBe(MAX_USER_THEMES);
  });
});
