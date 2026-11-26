// Reload on save (roadmap M5.7; docs/ux/plugins.md, "Reload on save"). A plugin loaded for development from a file this browser can read
// again (the File System Access file picker, in Chromium-based browsers) is watched for this visit: about once a second the file's time
// and size are looked at, and once a change has held for one look (a package written half-way is not read), the package is checked
// again exactly as a new file. One that passes takes the old one's place with the grant the person gave, so its frames start again,
// inert as any new frame; a capability the person was never shown holds the plugin, as any new version does. One that fails is not
// loaded: the last package that passed keeps running and the pane says why.
//
// Only development plugins are watched (nothing installed on a hub), nothing is watched in safe mode, and nothing is kept past the visit
// (a browser asks again before a page reads a file it was given in an earlier visit). Where the picker is missing (Firefox, Safari, the
// desktop app's WebKit), the file is chosen as before and loaded again by hand.
import { useSyncExternalStore } from 'react';
import type { PackageError } from './package';
import { type PluginStore, pluginStore } from './store';

export const POLL_MS = 1000;

/** A file this browser can read again (a `FileSystemFileHandle`). */
export interface WatchedFile { getFile(): Promise<Pick<File, 'size' | 'lastModified' | 'arrayBuffer'>> }
/** Why a changed file was not loaded: one of the package checks, a file that now holds another plugin, or one that cannot be read. */
export interface ReloadProblem { readonly code: PackageError | 'other-id' | 'unreadable'; readonly detail?: string }
export interface ReloadState {
  /** The plugins whose files are watched. */
  readonly watching: readonly string[];
  /** The last change that was not loaded, by plugin; gone once one is. */
  readonly problems: Readonly<Record<string, ReloadProblem>>;
}

type Store = Pick<PluginStore, 'state' | 'subscribe' | 'check' | 'replace'>;
interface Timers { set(f: () => void, ms: number): unknown; clear(t: unknown): void }
const realTimers: Timers = { set: (f, ms) => setInterval(f, ms), clear: (t) => clearInterval(t as ReturnType<typeof setInterval>) };

const stampOf = (f: Pick<File, 'size' | 'lastModified'>) => `${f.lastModified}:${f.size}`;

export function createDevReload(store: Store, timers: Timers = realTimers) {
  const watches = new Map<string, { file: WatchedFile; seen: string; pending?: string }>();
  let state: ReloadState = Object.freeze({ watching: Object.freeze([]), problems: Object.freeze({}) });
  const listeners = new Set<() => void>();
  let timer: unknown;
  let busy = false;
  const publish = (problems: Record<string, ReloadProblem> = { ...state.problems }) => {
    for (const id of Object.keys(problems)) if (!watches.has(id)) delete problems[id];
    state = Object.freeze({ watching: Object.freeze([...watches.keys()]), problems: Object.freeze(problems) });
    listeners.forEach((l) => l());
  };
  const setProblem = (id: string, p: ReloadProblem | undefined) => {
    const problems = { ...state.problems };
    if (p) problems[id] = Object.freeze(p); else delete problems[id];
    publish(problems);
  };
  const run = () => {
    if (watches.size > 0 && timer === undefined) timer = timers.set(() => { void tick(); }, POLL_MS);
    if (watches.size === 0 && timer !== undefined) { timers.clear(timer); timer = undefined; }
  };

  // A plugin removed (or no longer loaded) is not watched.
  store.subscribe(() => {
    const ids = new Set(store.state().plugins.filter((p) => p.dev).map((p) => p.plugin.manifest.id));
    let gone = false;
    for (const id of [...watches.keys()]) if (!ids.has(id)) { watches.delete(id); gone = true; }
    if (gone) { run(); publish(); }
  });

  async function look(id: string) {
    const w = watches.get(id);
    if (!w) return;
    let f: Awaited<ReturnType<WatchedFile['getFile']>>;
    try { f = await w.file.getFile(); } catch { if (state.problems[id]?.code !== 'unreadable') setProblem(id, { code: 'unreadable' }); return; }
    const stamp = stampOf(f);
    if (stamp === w.seen) { w.pending = undefined; return; }
    // A change is read once it held for one look, so a package still being written is not.
    if (stamp !== w.pending) { w.pending = stamp; return; }
    w.seen = stamp;
    w.pending = undefined;
    let r: Awaited<ReturnType<Store['check']>>;
    // The file changed again while it was read (the browser refuses to read a file that moved on): the next look sees that change.
    try { r = await store.check(f); } catch { return; }
    if (watches.get(id) !== w) return;
    if (!r.ok) { setProblem(id, { code: r.code, ...(r.detail !== undefined ? { detail: r.detail } : {}) }); return; }
    if (r.plugin.manifest.id !== id) { setProblem(id, { code: 'other-id', detail: r.plugin.manifest.id }); return; }
    if (r.text === undefined || !store.replace(r.plugin, r.text)) return;
    setProblem(id, undefined);
  }
  async function tick() {
    if (busy) return;
    busy = true;
    try { for (const id of [...watches.keys()]) await look(id); } finally { busy = false; }
  }

  return {
    state: () => state,
    subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
    /** Watches the file a development plugin was just loaded from (`loaded`: the file as it was read then). Nothing in safe mode. */
    watch(id: string, file: WatchedFile, loaded: Pick<File, 'size' | 'lastModified'>) {
      if (store.state().safe) return;
      watches.set(id, { file, seen: stampOf(loaded) });
      run();
      setProblem(id, undefined);
    },
    /** Stops watching (the plugin was loaded again from a file this browser cannot watch). */
    unwatch(id: string) {
      if (!watches.delete(id)) return;
      run();
      publish();
    },
    /** One look at every watched file now (tests; the timer calls it). */
    tick,
  };
}
export type DevReload = ReturnType<typeof createDevReload>;

/** Whether this browser can watch a file it was given (the File System Access file picker). */
export const canWatchFiles = (w: unknown = typeof window !== 'undefined' ? window : undefined): boolean =>
  typeof (w as { showOpenFilePicker?: unknown } | undefined)?.showOpenFilePicker === 'function';

type Picker = (o: { types: { description: string; accept: Record<string, string[]> }[]; multiple: false }) => Promise<WatchedFile[]>;

/** Asks the person for a package file with the picker that lets this page read it again: undefined when they cancel, `failed` when
 *  the picker would not open or read it (the window then offers the plain file input). */
export async function pickPackage(description: string, w: unknown = window): Promise<{ file: File; handle: WatchedFile } | 'failed' | undefined> {
  const pick = (w as { showOpenFilePicker?: Picker }).showOpenFilePicker;
  if (!pick) return 'failed';
  try {
    const [handle] = await pick.call(w, { types: [{ description, accept: { 'application/json': ['.json'] } }], multiple: false });
    if (!handle) return undefined;
    return { file: (await handle.getFile()) as File, handle };
  } catch (e) { return (e as { name?: unknown } | null)?.name === 'AbortError' ? undefined : 'failed'; }
}

export const devReload: DevReload = createDevReload(pluginStore);

/** What reload on save is doing, as React state. */
export function useDevReload(): ReloadState {
  return useSyncExternalStore(devReload.subscribe, devReload.state, devReload.state);
}
