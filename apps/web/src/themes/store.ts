// Which theme this browser shows, and the person's own themes (ADR-0068 rung 2). Remembered in this browser; when the browser cannot
// store, every choice still holds for the visit. Reading never throws. ADR-0113 step 4: the hub can keep the same for the person on all
// their devices (sync.ts); for that each of the person's themes has a lasting id across devices (`uid`) and the time of its last edit,
// a removed theme leaves a dated mark, and the choice has the time it was made. None of that is in a theme's file.
//
// The colour scheme follows the system, or is forced dark or light (the title bar's picker). Each scheme has its chosen theme,
// so "follow the system" can switch between, say, Sand by day and Slate by night. Another tab's change applies here at once.
import { type ThemeData, cloneLooks, deserialise, readTheme, writeTheme } from './file';
import { SAFE } from '../plugins/safe';
import {
  BUILT_IN, COLOR_TOKENS, type CatalogOrigin, DEFAULT_THEME, MAX_NAME, type Scheme, type StyleTarget, type Theme, type ThemeIssue, type Tokens,
  applyTheme, chromeOf, hasScheme, looksOf, schemesOf, textOf,
} from './tokens';

export type Mode = 'system' | Scheme;
export const MODE_KEY = 'agora.theme'; // as before this rung: absent = follow the system
export const CHOICE_KEY = 'agora.theme.choice.v1';
export const USER_KEY = 'agora.themes.v1';
export const MAX_USER_THEMES = 20;
// ADR-0113 step 4: themes removed here, by their lasting id and when (so another device does not bring them back), and when the choice
// was last made.
export const REMOVED_KEY = 'agora.themes.removed.v1';
export const CHOICE_AT_KEY = 'agora.theme.choice.at.v1';
/** The most removal marks kept: the oldest go first. */
export const MAX_REMOVED = 200;

type KV = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
export interface ThemeState { mode: Mode; choice: Record<Scheme, string>; user: readonly Theme[] }
// `warnings`: the theme was saved, but some of its text or controls are hard to see in the kit's colours (tokens.ts `themeWarnings`).
export type Saved = { ok: true; theme: Theme; warnings?: ThemeIssue[] } | { ok: false; issues: ThemeIssue[] } | { ok: false; full: true };

function read(kv: KV | undefined, key: string): string | null {
  try { return kv?.getItem(key) ?? null; } catch { return null; }
}
function write(kv: KV | undefined, key: string, value: string | undefined) {
  try { if (value === undefined) kv?.removeItem(key); else kv?.setItem(key, value); } catch { /* holds for this visit */ }
}
function json(text: string | null): unknown {
  try { return text ? JSON.parse(text) : undefined; } catch { return undefined; }
}

// ---- Lasting ids and edit times (ADR-0113 step 4) ---------------------------------------------------------------------------------
/** A theme's or a pack's id on every device of the person: 24 lowercase hex characters, made at random where it was first made. */
export const UID = /^[0-9a-f]{24}$/;
export function newUid(): string {
  const b = new Uint8Array(12);
  try { crypto.getRandomValues(b); } catch { for (let i = 0; i < b.length; i++) b[i] = Math.floor(Math.random() * 256); }
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
}
/** A time in milliseconds as kept: a whole number from 0, anything else 0. */
export const timeOf = (v: unknown): number => (typeof v === 'number' && Number.isSafeInteger(v) && v >= 0 ? v : 0);
export interface Removed { uid: string; at: number }
/** Removal marks as kept: valid ones, one per id (the latest), the newest `MAX_REMOVED`. */
export function removedOf(v: unknown): Removed[] {
  const by = new Map<string, number>();
  if (Array.isArray(v)) {
    for (const e of v.slice(0, MAX_REMOVED * 2)) {
      const uid = (e as { uid?: unknown })?.uid;
      if (typeof uid === 'string' && UID.test(uid)) by.set(uid, Math.max(by.get(uid) ?? 0, timeOf((e as { at?: unknown }).at)));
    }
  }
  return [...by].map(([uid, at]) => ({ uid, at })).sort((a, b) => b.at - a.at || (a.uid < b.uid ? -1 : 1)).slice(0, MAX_REMOVED);
}
/** One of the person's themes as another device sees it: its lasting id, when it was last edited, and the theme. */
export interface SyncTheme { uid: string; edited: number; theme: Theme }
/** What the hub keeps of this browser's themes: the person's themes, the removal marks, and the choice with the time it was made. In the
 *  choice a person's theme is named `u:<uid>` and a built-in theme by its id, since local ids differ between devices. */
export interface SyncSnapshot { themes: SyncTheme[]; removed: Removed[]; choice: Record<Scheme, string>; choiceAt: number }
/** What a merge gives this browser: each theme by lasting id (one this browser has keeps its place and local id), the marks, and the
 *  choice (`undefined` keeps this browser's). */
export interface SyncApply {
  themes: { uid: string; edited: number; data: ThemeData; catalog?: CatalogOrigin }[];
  removed: Removed[];
  choice?: Record<Scheme, string>;
  choiceAt: number;
  /** Another person's value replacing this browser's: the choice and its time are taken as given (the defaults when there is none). */
  exact?: boolean;
}
interface Meta { uid: string; edited: number }
// The lasting ids and edit times of the stored themes, by local id; a theme stored before this step gets a new lasting id.
function loadMeta(kv: KV | undefined, user: readonly Theme[]): { meta: Map<string, Meta>; fresh: boolean } {
  const meta = new Map<string, Meta>();
  let fresh = false;
  const list = json(read(kv, USER_KEY));
  const stored = new Map<string, { uid?: unknown; edited?: unknown }>();
  if (Array.isArray(list)) for (const e of list) { const id = (e as { id?: unknown })?.id; if (typeof id === 'string' && !stored.has(id)) stored.set(id, e as object); }
  const seen = new Set<string>();
  for (const u of user) {
    const e = stored.get(u.id);
    let uid = typeof e?.uid === 'string' && UID.test(e.uid) && !seen.has(e.uid) ? e.uid : undefined;
    if (!uid) { uid = newUid(); fresh = true; }
    seen.add(uid);
    meta.set(u.id, { uid, edited: timeOf(e?.edited) });
  }
  return { meta, fresh };
}
// Where an installed theme came from (ADR-0113 decision 6): the catalog's id rules (catalog.ts) and a whole version from 1. Anything
// else is dropped and the theme is kept as one of the person's own.
export function catalogOrigin(v: unknown): CatalogOrigin | undefined {
  const o = v as { id?: unknown; version?: unknown } | undefined;
  return o && typeof o === 'object' && typeof o.id === 'string' && /^[a-z0-9][a-z0-9-]{0,39}$/.test(o.id)
    && typeof o.version === 'number' && Number.isInteger(o.version) && o.version >= 1 && o.version <= 1_000_000
    ? { id: o.id, version: o.version } : undefined;
}

// What this browser had stored. Anything unreadable (another version's, edited by hand) falls back to the defaults; a stored user
// theme goes through the same checks as an imported file. Themes are stored as format-2 files; one stored before is a format-1 file.
export function loadState(kv: KV | undefined): ThemeState {
  const m = read(kv, MODE_KEY);
  const mode: Mode = m === 'dark' || m === 'light' ? m : 'system';
  const user: Theme[] = [];
  const list = json(read(kv, USER_KEY));
  if (Array.isArray(list)) {
    for (const e of list.slice(0, MAX_USER_THEMES)) {
      const id = (e as { id?: unknown })?.id;
      if (typeof id !== 'string' || !/^user-\d{1,6}$/.test(id) || user.some((u) => u.id === id)) continue;
      const p = readTheme((e as { file?: unknown }).file);
      const catalog = catalogOrigin((e as { catalog?: unknown }).catalog);
      if (p.ok) user.push(catalog ? { id, ...p.theme, catalog } : { id, ...p.theme });
    }
  }
  const c = json(read(kv, CHOICE_KEY)) as Partial<Record<Scheme, unknown>> | undefined;
  const all = [...BUILT_IN, ...user];
  const pick = (s: Scheme) => {
    const id = c && typeof c === 'object' ? c[s] : undefined;
    return all.some((t) => t.id === id && hasScheme(t, s)) ? (id as string) : DEFAULT_THEME[s];
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
  const loaded = loadMeta(env.kv, state.user);
  let meta = loaded.meta;
  let removed = removedOf(json(read(env.kv, REMOVED_KEY)));
  // A choice stored before it had a time counts as made at time 1 (any later choice is newer); the defaults, never chosen, as 0.
  const choiceTime = () => timeOf(Number(read(env.kv, CHOICE_AT_KEY))) || (read(env.kv, CHOICE_KEY) ? 1 : 0);
  let choiceAt = choiceTime();
  const now = () => Date.now();
  const listeners = new Set<() => void>();
  const notify = () => listeners.forEach((l) => l());
  const all = () => [...BUILT_IN, ...state.user];
  const byId = (id: string) => all().find((t) => t.id === id);
  const scheme = (): Scheme => (state.mode !== 'system' ? state.mode : env.prefersLight?.() ? 'light' : 'dark');
  const active = (): Theme => byId(state.choice[scheme()]) ?? (BUILT_IN.find((t) => t.id === DEFAULT_THEME[scheme()]) as Theme);
  const apply = () => { if (env.root) applyTheme(env.root, active(), scheme()); env.onApply?.(); };
  const set = (next: ThemeState) => { state = next; apply(); notify(); };
  const metaOf = (id: string): Meta => meta.get(id) ?? { uid: newUid(), edited: 0 };
  const persistUser = (user: readonly Theme[]) => {
    const next = new Map<string, Meta>();
    for (const u of user) next.set(u.id, metaOf(u.id));
    meta = next;
    write(env.kv, USER_KEY, JSON.stringify(user.map((u) => {
      const m = metaOf(u.id);
      return u.catalog ? { id: u.id, file: writeTheme(u, u.name), catalog: u.catalog, uid: m.uid, edited: m.edited } : { id: u.id, file: writeTheme(u, u.name), uid: m.uid, edited: m.edited };
    })));
  };
  // A theme made or changed here: a new one gets a lasting id; both get this moment as their last edit.
  const edited = (id: string) => { meta.set(id, { uid: meta.get(id)?.uid ?? newUid(), edited: now() }); };
  const persistRemoved = () => write(env.kv, REMOVED_KEY, removed.length ? JSON.stringify(removed) : undefined);
  // The choice's time moves only when the choice changes (removing a theme that was not chosen is no new choice).
  const persistChoice = (choice: Record<Scheme, string>, at?: number) => {
    choiceAt = at ?? (choice.dark !== state.choice.dark || choice.light !== state.choice.light ? now() : choiceAt);
    write(env.kv, CHOICE_AT_KEY, choiceAt ? String(choiceAt) : undefined);
    write(env.kv, CHOICE_KEY, choice.dark === DEFAULT_THEME.dark && choice.light === DEFAULT_THEME.light ? undefined : JSON.stringify(choice));
  };
  // The lasting ids of a theme stored before this step are kept at once, so they are the same the next time.
  if (loaded.fresh) persistUser(state.user);
  const freshId = () => `user-${Math.max(0, ...state.user.map((u) => Number(u.id.slice(5)))) + 1}`;
  // A theme becomes the choice of every scheme it has; a forced scheme it lacks moves to one it has, so the choice shows.
  const chosen = (theme: Theme) => {
    const choice = { ...state.choice };
    for (const s of schemesOf(theme)) choice[s] = theme.id;
    const mode: Mode = state.mode === 'system' || hasScheme(theme, state.mode) ? state.mode : theme.scheme;
    return { choice, mode };
  };
  // A new theme becomes the chosen one of its schemes and is shown.
  const adopt = (theme: Theme): Saved => {
    if (state.user.length >= MAX_USER_THEMES) return { ok: false, full: true };
    const user = [...state.user, theme];
    edited(theme.id);
    persistUser(user);
    const { choice, mode } = chosen(theme);
    persistChoice(choice);
    if (mode !== state.mode) write(env.kv, MODE_KEY, mode);
    set({ mode, choice, user });
    return { ok: true, theme };
  };

  const removeTheme = (id: string) => {
    if (!state.user.some((u) => u.id === id)) return;
    // A dated mark, so the hub's copy on the person's other devices goes too (unless it was edited there since).
    removed = removedOf([{ uid: metaOf(id).uid, at: now() }, ...removed]);
    persistRemoved();
    const user = state.user.filter((u) => u.id !== id);
    persistUser(user);
    const choice = { dark: state.choice.dark === id ? DEFAULT_THEME.dark : state.choice.dark, light: state.choice.light === id ? DEFAULT_THEME.light : state.choice.light };
    persistChoice(choice);
    set({ ...state, user, choice });
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
    reload: () => {
      const next = loadState(env.kv);
      meta = loadMeta(env.kv, next.user).meta;
      removed = removedOf(json(read(env.kv, REMOVED_KEY)));
      choiceAt = choiceTime();
      set(next);
    },
    // A new snapshot, so views that show which theme is on screen draw again.
    systemChanged: () => { if (state.mode === 'system') set({ ...state }); },
    setMode(mode: Mode) {
      write(env.kv, MODE_KEY, mode === 'system' ? undefined : mode);
      set({ ...state, mode });
    },
    // Use a theme: it becomes the choice of the schemes it has; a forced scheme it lacks moves to one it has.
    choose(id: string) {
      const t = byId(id);
      if (!t) return;
      const { choice, mode } = chosen(t);
      persistChoice(choice);
      if (mode !== state.mode) write(env.kv, MODE_KEY, mode);
      set({ ...state, mode, choice });
    },
    // A copy of any theme the person can change: both its schemes, its chrome and its text.
    copy(from: Theme, name: string): Saved {
      const looks = cloneLooks(looksOf(from));
      const scheme = from.scheme;
      return adopt({ id: freshId(), name: name.slice(0, MAX_NAME), scheme, tokens: looks[scheme]?.tokens ?? { ...from.tokens }, looks, chrome: { ...chromeOf(from) }, text: { ...textOf(from) } });
    },
    // A change to one of the person's themes, checked like an imported file: one that is refused changes nothing. `file` is the whole
    // theme as the editor holds it (a format-2 file); `tokens` changes the tokens of one scheme (its first, unless `scheme` says).
    update(id: string, patch: { name?: string; tokens?: Partial<Tokens>; scheme?: Scheme; file?: Record<string, unknown> }): Saved {
      const cur = state.user.find((u) => u.id === id);
      if (!cur) return { ok: false, issues: [{ code: 'invalid' }] };
      let file = patch.file ?? writeTheme(cur, cur.name);
      if (patch.tokens) {
        const s = patch.scheme ?? cur.scheme;
        const section = file[s] as { tokens?: Record<string, unknown> } | undefined;
        if (section) file = { ...file, [s]: { ...section, tokens: { ...section.tokens, ...pick(patch.tokens, 'color') } }, shape: { ...(file.shape as object), ...pick(patch.tokens, 'shape') }, text: { ...(file.text as object), ...pick(patch.tokens, 'font') } };
      }
      if (patch.name !== undefined) file = { ...file, name: patch.name };
      const p = readTheme(file);
      if (!p.ok) return p;
      const theme: Theme = cur.catalog ? { id, ...p.theme, catalog: cur.catalog } : { id, ...p.theme };
      const user = state.user.map((u) => (u.id === id ? theme : u));
      edited(id);
      persistUser(user);
      set({ ...state, user });
      return p.warnings ? { ok: true, theme, warnings: p.warnings } : { ok: true, theme };
    },
    import(text: string): Saved {
      const p = deserialise(text);
      if (!p.ok) return p;
      const r = adopt({ id: freshId(), ...p.theme });
      return r.ok && p.warnings ? { ...r, warnings: p.warnings } : r;
    },
    // How many more of the person's own themes fit.
    room: () => Math.max(0, MAX_USER_THEMES - state.user.length),
    // ADR-0113 decision 5: themes from a pack the person chose to add, checked again like an imported file and added to their own themes
    // all or none (when they do not all fit, or one is refused, nothing changes). Unlike an import, nothing is chosen: the theme on screen
    // stays, and the person picks an added one when they want it.
    add(themes: readonly ThemeData[]): { ok: true; themes: Theme[] } | { ok: false; full: true } | { ok: false; issues: ThemeIssue[] } {
      if (themes.length > MAX_USER_THEMES - state.user.length) return { ok: false, full: true };
      const added: Theme[] = [];
      let next = Math.max(0, ...state.user.map((u) => Number(u.id.slice(5))));
      for (const data of themes) {
        const p = readTheme(writeTheme(data, data.name));
        if (!p.ok) return p;
        added.push({ id: `user-${++next}`, ...p.theme });
      }
      const user = [...state.user, ...added];
      for (const a of added) edited(a.id);
      persistUser(user);
      set({ ...state, user });
      return { ok: true, themes: added };
    },
    // ADR-0113 decision 6: a theme from the hub's catalog, checked like an imported file and added to the person's own themes with
    // where it came from. It changes only through `upgrade`, when the person picks a newer version.
    install(text: string, origin: CatalogOrigin): Saved {
      const p = deserialise(text);
      if (!p.ok) return p;
      const r = adopt({ id: freshId(), ...p.theme, catalog: origin });
      return r.ok && p.warnings ? { ...r, warnings: p.warnings } : r;
    },
    // The newer version of an installed theme, in its place: the same theme of the person, chosen where it was (a scheme it no longer
    // has goes back to the default).
    upgrade(id: string, text: string, origin: CatalogOrigin): Saved {
      if (!state.user.some((u) => u.id === id)) return { ok: false, issues: [{ code: 'invalid' }] };
      const p = deserialise(text);
      if (!p.ok) return p;
      const theme: Theme = { id, ...p.theme, catalog: origin };
      const user = state.user.map((u) => (u.id === id ? theme : u));
      edited(id);
      persistUser(user);
      const keep = (s: Scheme) => (state.choice[s] === id && !hasScheme(theme, s) ? DEFAULT_THEME[s] : state.choice[s]);
      const choice = { dark: keep('dark'), light: keep('light') };
      persistChoice(choice);
      set({ ...state, user, choice });
      return p.warnings ? { ok: true, theme, warnings: p.warnings } : { ok: true, theme };
    },
    remove: removeTheme,
    // Back to the defaults: follow the system with Light and Dark. The person's themes are kept.
    reset() {
      write(env.kv, MODE_KEY, undefined);
      persistChoice({ ...DEFAULT_THEME });
      set({ ...state, mode: 'system', choice: { ...DEFAULT_THEME } });
    },
    // ---- ADR-0113 step 4: what the hub keeps for the person's other devices (sync.ts) ----
    // Removes a theme by its lasting id on the person's other devices: one this browser has goes as `remove` does; one it only heard of
    // (a synced theme it could not read, or one that did not fit) gets the dated mark alone.
    forget(uid: string) {
      if (!UID.test(uid)) return;
      const id = [...meta].find(([, m]) => m.uid === uid)?.[0];
      if (id) { removeTheme(id); return; }
      removed = removedOf([{ uid, at: now() }, ...removed]);
      persistRemoved();
      set({ ...state });
    },
    syncSnapshot(): SyncSnapshot {
      const ref = (s: Scheme) => {
        const id = state.choice[s];
        return BUILT_IN.some((b) => b.id === id) ? id : meta.has(id) ? `u:${metaOf(id).uid}` : DEFAULT_THEME[s];
      };
      return {
        themes: state.user.map((theme) => ({ uid: metaOf(theme.id).uid, edited: metaOf(theme.id).edited, theme })),
        removed: [...removed],
        choice: { dark: ref('dark'), light: ref('light') },
        choiceAt,
      };
    },
    // A merge's result, kept as given: no theme is checked again here (the merge read each with the theme rules), no edit time changes,
    // and at most `MAX_USER_THEMES` are kept (the merge leaves out what does not fit). A choice naming a theme this browser does not have
    // (or a scheme it lacks) keeps that scheme's choice.
    applySync(next: SyncApply) {
      const byUid = new Map([...meta].map(([id, m]) => [m.uid, id]));
      let n = Math.max(0, ...state.user.map((u) => Number(u.id.slice(5))));
      const user: Theme[] = [];
      const nextMeta = new Map<string, Meta>();
      for (const e of next.themes.slice(0, MAX_USER_THEMES)) {
        const id = byUid.get(e.uid) ?? `user-${++n}`;
        // The theme's own fields only: never an id or an origin that came along with them.
        const { name, scheme, tokens, looks, chrome, text } = e.data;
        const data: ThemeData = { name, scheme, tokens, looks, chrome, text };
        user.push(e.catalog ? { id, ...data, catalog: e.catalog } : { id, ...data });
        nextMeta.set(id, { uid: e.uid, edited: e.edited });
      }
      meta = nextMeta;
      persistUser(user);
      removed = removedOf(next.removed);
      persistRemoved();
      const all = [...BUILT_IN, ...user];
      let resolved = true;
      const local = (ref: string | undefined, s: Scheme, fallback: string) => {
        const id = ref?.startsWith('u:') ? [...nextMeta].find(([, m]) => `u:${m.uid}` === ref)?.[0] : ref;
        if (id && all.some((t) => t.id === id && hasScheme(t, s))) return id;
        resolved = false;
        return fallback;
      };
      const keep = (s: Scheme) => (all.some((t) => t.id === state.choice[s] && hasScheme(t, s)) ? state.choice[s] : DEFAULT_THEME[s]);
      let choice = { dark: keep('dark'), light: keep('light') };
      if (next.exact) choice = { ...DEFAULT_THEME };
      if (next.choice) choice = { dark: local(next.choice.dark, 'dark', choice.dark), light: local(next.choice.light, 'light', choice.light) };
      // A newer choice naming a theme this browser cannot show (one that waits, or that it cannot read) keeps its time out of here, so it
      // stays the newer one and this browser never writes its own over it.
      const at = next.exact ? next.choiceAt : resolved ? Math.max(choiceAt, next.choiceAt) : choiceAt;
      if (choice.dark !== state.choice.dark || choice.light !== state.choice.light || at !== choiceAt) persistChoice(choice, at);
      set({ ...state, user, choice });
    },
  };
}
export type ThemeStore = ReturnType<typeof createThemeStore>;

// The tokens of a format-1 change, sorted into the format-2 groups they belong to (by name, so a bad value is still judged as one).
function pick(tokens: Partial<Tokens>, group: 'color' | 'shape' | 'font'): Record<string, string> {
  const of = (k: string) => (k === 'font-ui' || k === 'font-mono' ? 'font' : (COLOR_TOKENS as readonly string[]).includes(k) ? 'color' : 'shape');
  return Object.fromEntries(Object.entries(tokens).filter(([k, v]) => v !== undefined && of(k) === group)) as Record<string, string>;
}
export type { ThemeData };

// ---- The page's store ---------------------------------------------------------------------------------------------------------

function storage(): KV | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
const LIGHT_QUERY = '(prefers-color-scheme: light)';
const browser = typeof window !== 'undefined' && typeof document !== 'undefined';

// In safe mode (ADR-0114, plugins/safe.ts) the window shows the built-in theme: nothing stored is read, and a choice made there holds for
// that visit only, so another tab's change never brings a stored theme back either.
export const themeStore: ThemeStore = createThemeStore(browser ? {
  kv: SAFE ? undefined : storage(),
  root: document.documentElement,
  prefersLight: () => !!window.matchMedia?.(LIGHT_QUERY).matches,
  // The terminals repaint on this; the variables have changed by the time it fires.
  onApply: () => window.dispatchEvent(new Event('agora-theme')),
} : {});

export function initTheme() {
  themeStore.apply();
  window.matchMedia?.(LIGHT_QUERY).addEventListener?.('change', themeStore.systemChanged);
  // Live reload: a theme edited or chosen in another tab of this browser shows here at once.
  window.addEventListener('storage', (e) => {
    if (e.key === null || e.key === MODE_KEY || e.key === CHOICE_KEY || e.key === USER_KEY || e.key === REMOVED_KEY || e.key === CHOICE_AT_KEY) themeStore.reload();
  });
}
