// Keeping the person's themes, packs and theme choice the same on all their devices (ADR-0113 decision 6, build step 4; docs/ux/themes.md
// "Keeping themes the same on your devices"). The hub keeps one value per person (`UserSettingsGet` / `UserSettingsSet`, key `themes`)
// as opaque bytes of at most 256 KiB and never reads it; this window writes it and checks everything in it again on load, with the same
// readers as a file (file.ts `readTheme`, pack.ts `readPack`). Something it cannot read is shown with its problems, never applied, and
// written back unchanged, so a newer window's themes survive an older one.
//
// The value, format 1 (JSON, written in one canonical order so that the same themes give the same bytes on every device):
//   { "agora-theme-sync": 1,
//     "themes":        [{ "uid", "edited", "file", "catalog"? }],   the person's themes, by lasting id, each a theme file
//     "themesRemoved": [{ "uid", "at" }],                          themes removed, and when
//     "packs":         [{ "uid", "added", "file" }],              packs the person imported, each a pack file
//     "packsRemoved":  [{ "uid", "at" }],
//     "choice":        { "dark", "light", "at" } }                each scheme's theme: a built-in id or `u:<uid>`
//
// Merging (`merge`), on every read: the union by lasting id. A theme both sides have takes the newer edit (this browser's on a tie). A
// removal mark removes a theme only when it is newer than the theme's last edit, so nothing edited here since is lost. A theme that does
// not fit this browser's 20 waits (it stays in the value for the devices that have room) and is shown. The choice is the newer one, the
// hub's on a tie; a choice made before this step counts as made at time 1, one never made (the defaults) as 0.
import type { agora } from '../gen/agora';
import { type ThemeData, readTheme, writeTheme } from './file';
import { SAFE } from '../plugins/safe';
import { type KeptPack, type PackStore, MAX_KEPT_PACKS, packStore } from './kept';
import { type PackIssue, readPack } from './pack';
import { type Removed, type SyncApply, type SyncSnapshot, type ThemeStore, MAX_USER_THEMES, UID, catalogOrigin, removedOf, themeStore, timeOf } from './store';
import { type CatalogOrigin, type Scheme, type Theme, type ThemeIssue, chromeOf, looksOf, textOf } from './tokens';

export const SYNC_KEY = 'agora-theme-sync';
export const SYNC_FORMAT = 1;
/** The hub's cap per person (ADR-0113 decision 6). */
export const MAX_SYNC_BYTES = 256 * 1024;
/** The most entries of each kind read from the value. */
export const MAX_SYNC_THEMES = 64;
export const MAX_SYNC_PACKS = 16;
/** This device's switch, "Keep my themes the same on my devices": absent means on. */
export const SETTING_KEY = 'agora.themes.sync.v1';
/** How long after the last change the window writes to the hub. */
export const DEBOUNCE_MS = 1500;

type Raw = Record<string, unknown>;
const isObject = (v: unknown): v is Raw => typeof v === 'object' && v !== null && !Array.isArray(v);

export interface RemoteTheme { uid: string; edited: number; data: ThemeData; catalog?: CatalogOrigin; raw: Raw }
export interface RemotePack { uid: string; added: number; kept: KeptPack; raw: Raw }
/** Something in the value this window cannot read: shown with its problems, never applied, written back as it was. */
export type Invalid =
  | { kind: 'theme'; uid: string; name?: string; issues: ThemeIssue[]; edited: number; raw: Raw }
  | { kind: 'pack'; uid: string; name?: string; issues: PackIssue[]; edited: number; raw: Raw };
export interface Remote {
  themes: RemoteTheme[];
  packs: RemotePack[];
  invalid: Invalid[];
  themesRemoved: Removed[];
  packsRemoved: Removed[];
  choice?: { dark: string; light: string; at: number };
}
export type ReadSync = { ok: true; remote: Remote } | { ok: false; newer: true };
export const EMPTY: Remote = { themes: [], packs: [], invalid: [], themesRemoved: [], packsRemoved: [] };

const nameOf = (file: unknown) => (isObject(file) && typeof file.name === 'string' ? file.name.slice(0, 60) : undefined);
const REF = /^(u:[0-9a-f]{24}|[a-z0-9][a-z0-9-]{0,39})$/;

/** Reads the hub's value. An empty value, or one that is not this format at all, reads as nothing kept yet; one made by a newer window
 *  is not read (and never written over). Every theme and pack is checked with the readers a file goes through. Never throws. */
export function readSync(text: string): ReadSync {
  let value: unknown;
  try { value = text ? JSON.parse(text) : undefined; } catch { return { ok: true, remote: EMPTY }; }
  if (!isObject(value)) return { ok: true, remote: EMPTY };
  const format = value[SYNC_KEY];
  if (typeof format !== 'number' || !Number.isInteger(format) || format < 1) return { ok: true, remote: EMPTY };
  if (format > SYNC_FORMAT) return { ok: false, newer: true };
  const remote: Remote = { themes: [], packs: [], invalid: [], themesRemoved: removedOf(value.themesRemoved), packsRemoved: removedOf(value.packsRemoved) };
  const seen = new Set<string>();
  // An entry with no lasting id cannot be merged or removed, so it is left out (no window writes one).
  const entries = (v: unknown, max: number) => (Array.isArray(v) ? v.slice(0, max) : []).filter((e): e is Raw => isObject(e) && typeof e.uid === 'string' && UID.test(e.uid) && !seen.has(e.uid) && (seen.add(e.uid), true));
  for (const e of entries(value.themes, MAX_SYNC_THEMES)) {
    const uid = e.uid as string;
    const edited = timeOf(e.edited);
    const r = readTheme(e.file);
    if (r.ok) {
      const catalog = catalogOrigin(e.catalog);
      remote.themes.push(catalog ? { uid, edited, data: r.theme, catalog, raw: e } : { uid, edited, data: r.theme, raw: e });
    } else remote.invalid.push({ kind: 'theme', uid, name: nameOf(e.file), issues: r.issues, edited, raw: e });
  }
  for (const e of entries(value.packs, MAX_SYNC_PACKS)) {
    const uid = e.uid as string;
    const added = timeOf(e.added);
    const r = isObject(e.file) ? readPack(e.file) : { ok: false as const, issues: [{ code: 'invalid' as const }] };
    if (r.ok) remote.packs.push({ uid, added, kept: { uid, added, file: e.file as Raw, pack: r.pack }, raw: e });
    else remote.invalid.push({ kind: 'pack', uid, name: nameOf(e.file), issues: r.issues, edited: added, raw: e });
  }
  const c = value.choice;
  if (isObject(c) && typeof c.dark === 'string' && typeof c.light === 'string' && REF.test(c.dark) && REF.test(c.light)) {
    remote.choice = { dark: c.dark, light: c.light, at: timeOf(c.at) };
  }
  return { ok: true, remote };
}

export interface Merged {
  /** What this browser keeps of themes and of packs. */
  themes: SyncApply;
  packs: { packs: KeptPack[]; removed: Removed[] };
  /** The value for the hub (`serialiseSync`). */
  value: Raw;
  /** Themes and packs from other devices that do not fit here: kept in the value, shown, not applied. */
  waiting: { kind: 'theme' | 'pack'; uid: string; name: string }[];
  invalid: Invalid[];
}

// A theme's own fields, without this browser's local id or its origin.
const dataOf = (t: Theme): ThemeData => ({ name: t.name, scheme: t.scheme, tokens: t.tokens, looks: looksOf(t), chrome: chromeOf(t), text: textOf(t) });
const byUid = <T extends { uid: string }>(a: T, b: T) => (a.uid < b.uid ? -1 : a.uid > b.uid ? 1 : 0);
const markOf = (marks: readonly Removed[]) => new Map(marks.map((m) => [m.uid, m.at]));

/** The union of this browser's themes and packs and the hub's (see the top of this file). Pure. */
export function merge(local: { themes: SyncSnapshot; packs: { packs: KeptPack[]; removed: Removed[] } }, remote: Remote): Merged {
  const waiting: Merged['waiting'] = [];
  // ---- Themes ----
  const themesRemoved = removedOf([...local.themes.removed, ...remote.themesRemoved]);
  const gone = markOf(themesRemoved);
  const removedSince = (uid: string, edited: number) => (gone.get(uid) ?? -1) > edited;
  const remoteBy = new Map(remote.themes.map((r) => [r.uid, r]));
  const localUids = new Set(local.themes.themes.map((t) => t.uid));
  type Kept = { uid: string; edited: number; data: ThemeData; catalog?: CatalogOrigin; raw?: Raw };
  const keptThemes: Kept[] = [];
  for (const t of local.themes.themes) {
    const r = remoteBy.get(t.uid);
    const k: Kept = r && r.edited > t.edited
      ? { uid: r.uid, edited: r.edited, data: r.data, catalog: r.catalog, raw: r.raw }
      : { uid: t.uid, edited: t.edited, data: dataOf(t.theme), catalog: t.theme.catalog, raw: r && r.edited === t.edited ? r.raw : undefined };
    if (!removedSince(k.uid, k.edited)) keptThemes.push(k);
  }
  // From other devices: the newest first, while there is room; the rest wait.
  const arriving = remote.themes.filter((r) => !localUids.has(r.uid) && !removedSince(r.uid, r.edited)).sort((a, b) => b.edited - a.edited || byUid(a, b));
  const room = Math.max(0, MAX_USER_THEMES - keptThemes.length);
  for (const r of arriving.slice(0, room)) keptThemes.push({ uid: r.uid, edited: r.edited, data: r.data, catalog: r.catalog, raw: r.raw });
  const waitingThemes = arriving.slice(room);
  for (const r of waitingThemes) waiting.push({ kind: 'theme', uid: r.uid, name: r.data.name });

  // ---- Packs ----
  const packsRemoved = removedOf([...local.packs.removed, ...remote.packsRemoved]);
  const packGone = markOf(packsRemoved);
  const packRemoved = (uid: string, added: number) => (packGone.get(uid) ?? -1) > added;
  const localPackUids = new Set(local.packs.packs.map((p) => p.uid));
  const keptPacks = local.packs.packs.filter((p) => !packRemoved(p.uid, p.added));
  const arrivingPacks = remote.packs.filter((r) => !localPackUids.has(r.uid) && !packRemoved(r.uid, r.added)).sort((a, b) => b.added - a.added || byUid(a, b));
  const packRoom = Math.max(0, MAX_KEPT_PACKS - keptPacks.length);
  for (const r of arrivingPacks.slice(0, packRoom)) keptPacks.push(r.kept);
  const waitingPacks = arrivingPacks.slice(packRoom);
  for (const r of waitingPacks) waiting.push({ kind: 'pack', uid: r.uid, name: r.kept.pack.name });
  const remotePackRaw = new Map(remote.packs.map((r) => [r.uid, r.raw]));

  // ---- What this window cannot read: written back unless removed since, never applied ----
  const invalid = remote.invalid.filter((i) => !(i.kind === 'theme' ? localUids.has(i.uid) || removedSince(i.uid, i.edited) : localPackUids.has(i.uid) || packRemoved(i.uid, i.edited)));

  // ---- The choice: the newer one; on a tie the hub's, so two devices never write their own over each other's ----
  const remoteNewer = !!remote.choice && remote.choice.at >= local.themes.choiceAt;
  const choice = remoteNewer && remote.choice ? remote.choice : { ...local.themes.choice, at: local.themes.choiceAt };

  // ---- The value, in one order whatever the order on each device ----
  const themeEntry = (k: Kept): Raw => k.raw ?? (k.catalog
    ? { uid: k.uid, edited: k.edited, file: writeTheme(k.data, k.data.name), catalog: k.catalog }
    : { uid: k.uid, edited: k.edited, file: writeTheme(k.data, k.data.name) });
  const themes = [
    ...keptThemes.map((k) => ({ uid: k.uid, entry: themeEntry(k) })),
    ...waitingThemes.map((r) => ({ uid: r.uid, entry: r.raw })),
    ...invalid.filter((i) => i.kind === 'theme').map((i) => ({ uid: i.uid, entry: i.raw })),
  ].sort(byUid).map((x) => x.entry);
  const packs = [
    ...keptPacks.map((p) => ({ uid: p.uid, entry: remotePackRaw.get(p.uid) ?? { uid: p.uid, added: p.added, file: p.file } })),
    ...waitingPacks.map((r) => ({ uid: r.uid, entry: r.raw })),
    ...invalid.filter((i) => i.kind === 'pack').map((i) => ({ uid: i.uid, entry: i.raw })),
  ].sort(byUid).map((x) => x.entry);
  const value: Raw = { [SYNC_KEY]: SYNC_FORMAT, themes, themesRemoved, packs, packsRemoved, choice: { dark: choice.dark, light: choice.light, at: choice.at } };

  return {
    themes: {
      themes: keptThemes.map((k) => (k.catalog ? { uid: k.uid, edited: k.edited, data: k.data, catalog: k.catalog } : { uid: k.uid, edited: k.edited, data: k.data })),
      removed: themesRemoved,
      choice: remoteNewer && remote.choice ? { dark: remote.choice.dark, light: remote.choice.light } : undefined,
      choiceAt: Math.max(local.themes.choiceAt, remote.choice?.at ?? 0),
    },
    packs: { packs: keptPacks, removed: packsRemoved },
    value,
    waiting,
    invalid,
  };
}

export const serialiseSync = (value: Raw) => JSON.stringify(value);
const bytes = (text: string) => new TextEncoder().encode(text);

// Whether applying a merge would change this browser: the same themes (by id, edit and order), packs, marks and choice change nothing.
function changesLocal(local: { themes: SyncSnapshot; packs: { packs: KeptPack[]; removed: Removed[] } }, m: Merged): boolean {
  const themeKey = (list: { uid: string; edited: number }[]) => list.map((t) => `${t.uid}@${t.edited}`).join(',');
  const marks = (list: readonly Removed[]) => list.map((r) => `${r.uid}@${r.at}`).join(',');
  const choiceChanges = !!m.themes.choice && (m.themes.choice.dark !== local.themes.choice.dark || m.themes.choice.light !== local.themes.choice.light);
  return themeKey(local.themes.themes) !== themeKey(m.themes.themes)
    || marks(local.themes.removed) !== marks(m.themes.removed)
    || local.packs.packs.map((p) => p.uid).join(',') !== m.packs.packs.map((p) => p.uid).join(',')
    || marks(local.packs.removed) !== marks(m.packs.removed)
    || choiceChanges
    || m.themes.choiceAt !== local.themes.choiceAt;
}

// ---- The engine: when this window reads and writes -------------------------------------------------------------------------------

type Body = { userSettingsGet: { key: string } } | { userSettingsSet: { key: string; value: Uint8Array; baseVersion: number } };
/** How the window asks the hub (live.ts `command` with no machine named). */
export type SyncAsk = (body: Body) => Promise<agora.v1.CommandOk.$Properties>;

/** Where syncing stands, as Settings and the theme editor say it. */
export type SyncStatus =
  | 'off' // the person turned it off on this device
  | 'safe' // safe mode: nothing synced
  | 'local' // no hub (the demo window, or not signed in yet)
  | 'older' // the hub does not keep settings (an older hub): themes stay in this browser
  | 'syncing'
  | 'synced'
  | 'too-large' // more than 256 KiB: nothing written, this browser keeps everything
  | 'newer' // the hub's value was written by a newer window: not read, never written over
  | 'error'; // the hub could not be reached or refused; tried again at the next change or visit
export interface SyncView { status: SyncStatus; waiting: Merged['waiting']; invalid: Invalid[] }

type KV = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
export interface SyncEnv {
  themes: ThemeStore;
  packs: PackStore;
  kv?: KV;
  safe?: boolean;
  /** The cap (the hub's, `MAX_SYNC_BYTES`, unless a test says less). */
  maxBytes?: number;
  debounceMs?: number;
  setTimer?: (f: () => void, ms: number) => unknown;
  clearTimer?: (t: unknown) => void;
}

const num = (v: unknown) => (typeof v === 'number' ? v : typeof v === 'string' ? Number(v) : typeof v === 'object' && v !== null && 'toNumber' in v ? (v as { toNumber: () => number }).toNumber() : 0);
const codeOf = (e: unknown) => (typeof (e as { code?: unknown })?.code === 'string' ? (e as { code: string }).code : '');
const paramsOf = (e: unknown) => ((e as { params?: Record<string, string> })?.params ?? {});
/** An older hub does not know the commands: it decodes one as a request with no command, which its routing refuses before anything
 *  else as `auth.forbidden` naming the action `None` (crates/agora-hub/tests/user_settings.rs checks the answer), or as an unsupported
 *  message. Anything else is a hub that knows them but could not answer. */
export function isOlderHub(e: unknown): boolean {
  const code = codeOf(e);
  const p = paramsOf(e);
  return (code === 'auth.forbidden' && (p.action === 'None' || !p.action)) || (code === 'protocol.invalid_message' && p.field === 'command') || code === 'protocol.capability_missing';
}

export function createThemeSync(env: SyncEnv) {
  const setTimer = env.setTimer ?? ((f, ms) => setTimeout(f, ms));
  const clearTimer = env.clearTimer ?? ((t) => clearTimeout(t as ReturnType<typeof setTimeout>));
  const debounceMs = env.debounceMs ?? DEBOUNCE_MS;
  const readSetting = () => { try { return env.kv?.getItem(SETTING_KEY) !== 'off'; } catch { return true; } };
  let enabled = readSetting();
  let ask: SyncAsk | undefined;
  let hub = false; // whether this window has a hub at all
  let older = false;
  // What the hub last said: its version, its value's text, and what was read from it. Nothing is written before a read succeeded.
  let version = 0;
  let remoteText: string | undefined;
  let remote: Remote | undefined;
  let view: SyncView = { status: env.safe ? 'safe' : 'local', waiting: [], invalid: [] };
  let timer: unknown;
  let applying = false;
  let chain: Promise<void> = Promise.resolve();
  const listeners = new Set<() => void>();
  const show = (next: Partial<SyncView>) => { view = { ...view, ...next }; listeners.forEach((l) => l()); };
  const active = () => !env.safe && enabled && !!ask && !older;
  // Where it stands before a read: why it does not sync, or that it is about to.
  const standing = (): SyncStatus => (env.safe ? 'safe' : !enabled ? 'off' : !hub ? 'local' : older ? 'older' : 'syncing');
  const local = () => ({ themes: env.themes.syncSnapshot(), packs: env.packs.syncSnapshot() });
  const run = (f: () => Promise<void>) => { chain = chain.then(f, f); return chain; };

  // Merges with what the hub last said, keeps the result here when it changes anything, and writes the value when it differs.
  async function push(attempt: number): Promise<void> {
    if (!active() || !remote || !ask) return;
    const here = local();
    const m = merge(here, remote);
    if (changesLocal(here, m)) {
      applying = true;
      try { env.themes.applySync(m.themes); env.packs.applySync(m.packs); } finally { applying = false; }
    }
    const text = serialiseSync(m.value);
    if (text === remoteText) { show({ status: 'synced', waiting: m.waiting, invalid: m.invalid }); return; }
    if (bytes(text).length > (env.maxBytes ?? MAX_SYNC_BYTES)) { show({ status: 'too-large', waiting: m.waiting, invalid: m.invalid }); return; }
    show({ status: 'syncing', waiting: m.waiting, invalid: m.invalid });
    try {
      const r = (await ask({ userSettingsSet: { key: 'themes', value: bytes(text), baseVersion: version } })).userSettings;
      version = num(r?.version);
      remoteText = text;
      const again = readSync(text);
      remote = again.ok ? again.remote : remote;
      show({ status: 'synced' });
    } catch (e) {
      const code = codeOf(e);
      // Another window wrote first: read again, merge, write again (a few times at most).
      if (code === 'settings.stale' && attempt < 3) { await pull(attempt + 1); return; }
      show({ status: code === 'settings.too_large' ? 'too-large' : 'error' });
    }
  }

  async function pull(attempt = 0): Promise<void> {
    if (!active() || !ask) return;
    show({ status: 'syncing' });
    let r: agora.v1.IUserSettingsValue | null | undefined;
    try { r = (await ask({ userSettingsGet: { key: 'themes' } })).userSettings; } catch (e) {
      if (isOlderHub(e)) { older = true; show({ status: 'older', waiting: [], invalid: [] }); } else show({ status: 'error' });
      return;
    }
    const raw = r?.value;
    // Bytes as protobufjs gives them (a Uint8Array, or a Buffer, possibly of another realm).
    const text = raw && ArrayBuffer.isView(raw) ? new TextDecoder().decode(raw) : '';
    const read = readSync(text);
    if (!read.ok) { remote = undefined; show({ status: 'newer', waiting: [], invalid: [] }); return; }
    version = num(r?.version);
    remoteText = text;
    remote = read.remote;
    await push(attempt);
  }

  // A change here is written once the changes settle; when the last read failed (the hub could not be reached), it reads first.
  const changed = () => {
    if (applying || !active()) return;
    if (timer !== undefined) clearTimer(timer);
    timer = setTimer(() => { timer = undefined; void run(() => (remote ? push(0) : pull())); }, debounceMs);
  };
  env.themes.subscribe(changed);
  env.packs.subscribe(changed);

  return {
    view: () => view,
    subscribe: (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; },
    enabled: () => enabled,
    /** The hub this window is connected to (`undefined`: none yet, or none at all; `hasHub` says which). A new connection reads first. */
    attach(next: SyncAsk | undefined, hasHub = !!next) {
      if (next === ask && hasHub === hub) return;
      ask = next;
      hub = hasHub;
      older = false;
      remote = undefined;
      remoteText = undefined;
      version = 0;
      if (timer !== undefined) { clearTimer(timer); timer = undefined; }
      show({ status: standing(), waiting: [], invalid: [] });
      if (active()) void run(() => pull());
    },
    /** Reads again (the window came back into view): another device may have changed something. */
    refresh() { if (active()) void run(() => pull()); return chain; },
    setEnabled(on: boolean) {
      if (env.safe) return;
      enabled = on;
      try { if (on) env.kv?.removeItem(SETTING_KEY); else env.kv?.setItem(SETTING_KEY, 'off'); } catch { /* holds for this visit */ }
      if (!on) { remote = undefined; remoteText = undefined; if (timer !== undefined) { clearTimer(timer); timer = undefined; } }
      show({ status: standing(), waiting: [], invalid: [] });
      if (active()) void run(() => pull());
    },
    /** Settles every read and write started so far (tests). */
    settled: () => chain,
  };
}
export type ThemeSync = ReturnType<typeof createThemeSync>;
export type { Scheme };

// ---- The page's engine ----------------------------------------------------------------------------------------------------------
function storage(): KV | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}
// Safe mode (ADR-0114) never syncs: nothing is read from the hub and nothing is written to it.
export const themeSync: ThemeSync = createThemeSync({ themes: themeStore, packs: packStore, kv: SAFE ? undefined : storage(), safe: SAFE });
