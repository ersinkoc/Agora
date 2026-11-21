import { chooseSetting, chosenSettings, noSettings, usableSettings, type LaunchSettingChoice, type SettingsState } from './launchSettingsModel';

// ADR-0066 decision 3: the start window remembers the last model and effort chosen for each kind on each machine, on this device only
// (localStorage, guarded: private windows and blocked storage just remember nothing). It is a convenience, never an authority: what it
// gives back is checked against what the machine lists NOW (`recall`), so a value the machine no longer takes is dropped, never sent.
const KEY = 'agora.start.settings.v1';
const MAX_MACHINES = 64;
const MAX_KINDS = 32;

type Memory = Record<string, Record<string, Record<string, string>>>;
type Store = Pick<Storage, 'getItem' | 'setItem'>;

function defaultStore(): Store | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}

const isRecord = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);

function read(store: Store | undefined): Memory {
  try {
    const raw = store?.getItem(KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    if (!isRecord(parsed)) return {};
    const out: Memory = {};
    for (const [node, kinds] of Object.entries(parsed)) {
      if (!isRecord(kinds)) continue;
      const k: Record<string, Record<string, string>> = {};
      for (const [kind, values] of Object.entries(kinds)) {
        if (!isRecord(values)) continue;
        const v: Record<string, string> = {};
        for (const [id, value] of Object.entries(values)) if (typeof value === 'string') v[id] = value;
        k[kind] = v;
      }
      out[node] = k;
    }
    return out;
  } catch { return {}; }
}

function write(store: Store | undefined, memory: Memory) {
  try { store?.setItem(KEY, JSON.stringify(memory)); } catch { /* full or blocked: nothing is remembered */ }
}

/** What was last chosen for `kind` on `node`, cleaned against what the machine lists now: a setting it no longer has, or a value outside
 * its list (or not a plain token), is left out. Empty when nothing usable remains: the agent's own default. */
export function recall(node: string, kind: string, specs: readonly LaunchSettingChoice[], store: Store | undefined = defaultStore()): SettingsState {
  const saved = read(store)[node]?.[kind];
  if (!saved) return noSettings;
  let state: SettingsState = noSettings;
  for (const spec of usableSettings(specs)) {
    const value = saved[spec.id];
    if (value !== undefined) state = chooseSetting(state, spec, value);
  }
  return state;
}

/** Remembers what a start just sent for `kind` on `node` (the pairs it signed). Starting with the agent's own defaults forgets the choice. */
export function remember(node: string, kind: string, sent: readonly (readonly [string, string])[], specs: readonly LaunchSettingChoice[], store: Store | undefined = defaultStore()): void {
  if (!node || !kind) return;
  const memory = read(store);
  const mine = { ...(memory[node] ?? {}) };
  const pairs = chosenSettings(Object.fromEntries(sent), specs);
  if (pairs.length === 0) delete mine[kind]; else { delete mine[kind]; mine[kind] = Object.fromEntries(pairs); }
  const kinds = Object.entries(mine).slice(-MAX_KINDS);
  const next: Memory = { ...memory };
  delete next[node];
  if (kinds.length > 0) next[node] = Object.fromEntries(kinds);
  write(store, Object.fromEntries(Object.entries(next).slice(-MAX_MACHINES)));
}

// What this device started an agent with, kept per agent so the agent's header can say it read-only (the agent never reports effort).
const STARTED_KEY = 'agora.started.settings.v1';
const MAX_STARTED = 200;

/** Remembers the settings a start sent for the agent it created (hub-wide key `<node>/<agent>`). */
export function rememberStarted(agentKey: string, sent: readonly (readonly [string, string])[], store: Store | undefined = defaultStore()): void {
  if (!agentKey || sent.length === 0) return;
  try {
    const raw = store?.getItem(STARTED_KEY);
    const held: unknown = raw ? JSON.parse(raw) : {};
    const all = isRecord(held) ? Object.entries(held) : [];
    const next = [...all.filter(([k]) => k !== agentKey), [agentKey, Object.fromEntries(sent)] as const].slice(-MAX_STARTED);
    store?.setItem(STARTED_KEY, JSON.stringify(Object.fromEntries(next)));
  } catch { /* nothing is remembered */ }
}

/** The settings this device started `agentKey` with: id and value pairs, empty when unknown (started elsewhere, or by an older window). */
export function startedWith(agentKey: string, store: Store | undefined = defaultStore()): [string, string][] {
  try {
    const raw = store?.getItem(STARTED_KEY);
    const held: unknown = raw ? JSON.parse(raw) : {};
    const mine = isRecord(held) ? held[agentKey] : undefined;
    if (!isRecord(mine)) return [];
    return Object.entries(mine).flatMap(([id, v]) => (typeof v === 'string' && /^[a-z0-9-]{1,32}$/.test(id) ? [[id, v] as [string, string]] : []));
  } catch { return []; }
}
