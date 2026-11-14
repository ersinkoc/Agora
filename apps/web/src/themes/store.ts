// Which theme this browser shows, and the person's own themes (ADR-0068 rung 2). Remembered in this browser only (the hub is not
// involved); when the browser cannot store, every choice still holds for the visit. Reading never throws.
//
// The colour scheme follows the system, or is forced dark or light (the title bar's picker). Each scheme has its chosen theme,
// so "follow the system" can switch between, say, Sand by day and Slate by night. Another tab's change applies here at once.
import {
  BUILT_IN, DEFAULT_THEME, MAX_NAME, type Scheme, type StyleTarget, type Theme, type ThemeIssue, type Tokens,
  applyTheme, deserialiseTheme, parseTheme, themeFile,
} from './tokens';

export type Mode = 'system' | Scheme;
export const MODE_KEY = 'agora.theme'; // as before this rung: absent = follow the system
export const CHOICE_KEY = 'agora.theme.choice.v1';
export const USER_KEY = 'agora.themes.v1';
export const MAX_USER_THEMES = 20;

type KV = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
export interface ThemeState { mode: Mode; choice: Record<Scheme, string>; user: readonly Theme[] }
export type Saved = { ok: true; theme: Theme } | { ok: false; issues: ThemeIssue[] } | { ok: false; full: true };

function read(kv: KV | undefined, key: string): string | null {
  try { return kv?.getItem(key) ?? null; } catch { return null; }
}
function write(kv: KV | undefined, key: string, value: string | undefined) {
  try { if (value === undefined) kv?.removeItem(key); else kv?.setItem(key, value); } catch { /* holds for this visit */ }
}
function json(text: string | null): unknown {
  try { return text ? JSON.parse(text) : undefined; } catch { return undefined; }
}

// What this browser had stored. Anything unreadable (another version's, edited by hand) falls back to the defaults; a stored user
// theme goes through the same checks as an imported file.
export function loadState(kv: KV | undefined): ThemeState {
  const m = read(kv, MODE_KEY);
  const mode: Mode = m === 'dark' || m === 'light' ? m : 'system';
  const user: Theme[] = [];
  const list = json(read(kv, USER_KEY));
  if (Array.isArray(list)) {
    for (const e of list.slice(0, MAX_USER_THEMES)) {
      const id = (e as { id?: unknown })?.id;
      if (typeof id !== 'string' || !/^user-\d{1,6}$/.test(id) || user.some((u) => u.id === id)) continue;
      const p = parseTheme((e as { file?: unknown }).file);
      if (p.ok) user.push({ id, name: p.name, scheme: p.scheme, tokens: p.tokens });
    }
  }
  const c = json(read(kv, CHOICE_KEY)) as Partial<Record<Scheme, unknown>> | undefined;
  const all = [...BUILT_IN, ...user];
  const pick = (s: Scheme) => {
    const id = c && typeof c === 'object' ? c[s] : undefined;
    return all.some((t) => t.id === id && t.scheme === s) ? (id as string) : DEFAULT_THEME[s];
  };
  return { mode, choice: { dark: pick('dark'), light: pick('light') }, user };
}

export interface ThemeStoreEnv {
  kv?: KV;
  root?: StyleTarget; // where the variables go (the page's <html>); none in a test that only checks state
  prefersLight?: () => boolean;
  onApply?: () => void; // after the page changed (the terminals repaint)
}

export function createThemeStore(env: ThemeStoreEnv = {}) {
  let state = loadState(env.kv);
  const listeners = new Set<() => void>();
  const notify = () => listeners.forEach((l) => l());
  const all = () => [...BUILT_IN, ...state.user];
  const byId = (id: string) => all().find((t) => t.id === id);
  const scheme = (): Scheme => (state.mode !== 'system' ? state.mode : env.prefersLight?.() ? 'light' : 'dark');
  const active = (): Theme => byId(state.choice[scheme()]) ?? (BUILT_IN.find((t) => t.id === DEFAULT_THEME[scheme()]) as Theme);
  const apply = () => { if (env.root) applyTheme(env.root, active()); env.onApply?.(); };
  const set = (next: ThemeState) => { state = next; apply(); notify(); };
  const persistUser = (user: readonly Theme[]) => write(env.kv, USER_KEY, JSON.stringify(user.map((u) => ({ id: u.id, file: themeFile(u, u.name) }))));
  const persistChoice = (choice: Record<Scheme, string>) =>
    write(env.kv, CHOICE_KEY, choice.dark === DEFAULT_THEME.dark && choice.light === DEFAULT_THEME.light ? undefined : JSON.stringify(choice));
  const freshId = () => `user-${Math.max(0, ...state.user.map((u) => Number(u.id.slice(5)))) + 1}`;
  // A new theme becomes the chosen one of its scheme and is shown.
  const adopt = (theme: Theme): Saved => {
    if (state.user.length >= MAX_USER_THEMES) return { ok: false, full: true };
    const user = [...state.user, theme];
    persistUser(user);
    const choice = { ...state.choice, [theme.scheme]: theme.id };
    persistChoice(choice);
    const mode = state.mode === 'system' ? 'system' : theme.scheme;
    if (mode !== state.mode) write(env.kv, MODE_KEY, mode);
    set({ mode, choice, user });
    return { ok: true, theme };
  };

  return {
    state: () => state,
    subscribe: (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; },
    themes: all,
    byId,
    scheme,
    active,
    apply,
    // Another tab changed what is stored, or the system switched between dark and light.
    reload: () => set(loadState(env.kv)),
    // A new snapshot, so views that show which theme is on screen draw again.
    systemChanged: () => { if (state.mode === 'system') set({ ...state }); },
    setMode(mode: Mode) {
      write(env.kv, MODE_KEY, mode === 'system' ? undefined : mode);
      set({ ...state, mode });
    },
    // Use a theme: it becomes its scheme's choice; a forced scheme moves to it so the choice shows.
    choose(id: string) {
      const t = byId(id);
      if (!t) return;
      const choice = { ...state.choice, [t.scheme]: t.id };
      persistChoice(choice);
      const mode = state.mode === 'system' ? 'system' : t.scheme;
      if (mode !== state.mode) write(env.kv, MODE_KEY, mode);
      set({ ...state, mode, choice });
    },
    // A copy of any theme the person can change.
    copy(from: Theme, name: string): Saved {
      return adopt({ id: freshId(), name: name.slice(0, MAX_NAME), scheme: from.scheme, tokens: { ...from.tokens } });
    },
    // A change to one of the person's themes, checked like an imported file: one that is refused changes nothing.
    update(id: string, patch: { name?: string; tokens?: Partial<Tokens> }): Saved {
      const cur = state.user.find((u) => u.id === id);
      if (!cur) return { ok: false, issues: [{ code: 'invalid' }] };
      const p = parseTheme(themeFile({ scheme: cur.scheme, tokens: { ...cur.tokens, ...patch.tokens } }, patch.name ?? cur.name));
      if (!p.ok) return p;
      const theme: Theme = { id, name: p.name, scheme: p.scheme, tokens: p.tokens };
      const user = state.user.map((u) => (u.id === id ? theme : u));
      persistUser(user);
      set({ ...state, user });
      return { ok: true, theme };
    },
    import(text: string): Saved {
      const p = deserialiseTheme(text);
      if (!p.ok) return p;
      return adopt({ id: freshId(), name: p.name, scheme: p.scheme, tokens: p.tokens });
    },
    remove(id: string) {
      if (!state.user.some((u) => u.id === id)) return;
      const user = state.user.filter((u) => u.id !== id);
      persistUser(user);
      const choice = { dark: state.choice.dark === id ? DEFAULT_THEME.dark : state.choice.dark, light: state.choice.light === id ? DEFAULT_THEME.light : state.choice.light };
      persistChoice(choice);
      set({ ...state, user, choice });
    },
    // Back to the defaults: follow the system with Light and Dark. The person's themes are kept.
    reset() {
      write(env.kv, MODE_KEY, undefined);
      write(env.kv, CHOICE_KEY, undefined);
      set({ ...state, mode: 'system', choice: { ...DEFAULT_THEME } });
    },
  };
}
export type ThemeStore = ReturnType<typeof createThemeStore>;

// ---- The page's store ---------------------------------------------------------------------------------------------------------

function storage(): KV | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
const LIGHT_QUERY = '(prefers-color-scheme: light)';
const browser = typeof window !== 'undefined' && typeof document !== 'undefined';

export const themeStore: ThemeStore = createThemeStore(browser ? {
  kv: storage(),
  root: document.documentElement,
  prefersLight: () => !!window.matchMedia?.(LIGHT_QUERY).matches,
  // The terminals repaint on this; the variables have changed by the time it fires.
  onApply: () => window.dispatchEvent(new Event('agora-theme')),
} : {});

export function initTheme() {
  themeStore.apply();
  window.matchMedia?.(LIGHT_QUERY).addEventListener?.('change', themeStore.systemChanged);
  // Live reload: a theme edited or chosen in another tab of this browser shows here at once.
  window.addEventListener('storage', (e) => { if (e.key === null || e.key === MODE_KEY || e.key === CHOICE_KEY || e.key === USER_KEY) themeStore.reload(); });
}
