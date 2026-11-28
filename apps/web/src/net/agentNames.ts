// The names a person gave agents in this window (the pencil on a row of the agent list, F2 on a row). A name is this viewer's own
// label, like the group choice: kept in this browser only, never sent to the hub, a machine or another agent. Every place the window
// names an agent (the list, tabs, tiles, cards, the team chat, the board) shows it, because `tiles()` applies it. Storage that is
// blocked or full only means the names hold for this visit.
//
// The value, format 1 (JSON): { "v": 1, "names": { "<node>/<agent>": { "name", "at" } } }. `at` is when it was set (ms), so a later
// sync between devices can take the newer of two; the oldest entries go first past `MAX_AGENT_NAMES`.
import { useSyncExternalStore } from 'react';
import { plainText } from './agentKinds';

const KEY = 'agora.agentNames.v1';
/** The longest name kept (characters); longer text is cut. */
export const MAX_AGENT_NAME = 40;
/** The most names kept; setting one more forgets the oldest. */
export const MAX_AGENT_NAMES = 500;

interface Entry { name: string; at: number }
type Names = ReadonlyMap<string, Entry>;

let current: Names | undefined;
let version = 0;

/** A name as it is kept: one line of plain text, at most `MAX_AGENT_NAME` characters; empty means "no name of my own". */
export const cleanAgentName = (raw: string): string => plainText(raw, MAX_AGENT_NAME);

export function parseAgentNames(raw: unknown): Map<string, Entry> {
  const out = new Map<string, Entry>();
  if (typeof raw !== 'string') return out;
  let data: unknown;
  try { data = JSON.parse(raw); } catch { return out; }
  if (!data || typeof data !== 'object' || (data as { v?: unknown }).v !== 1) return out;
  const names = (data as { names?: unknown }).names;
  if (!names || typeof names !== 'object') return out;
  for (const [id, e] of Object.entries(names as Record<string, unknown>)) {
    if (!e || typeof e !== 'object' || id.length > 256) continue;
    const name = typeof (e as Entry).name === 'string' ? cleanAgentName((e as Entry).name) : '';
    const at = Number((e as Entry).at);
    if (name) out.set(id, { name, at: Number.isFinite(at) ? at : 0 });
  }
  return trim(out);
}

function trim(m: Map<string, Entry>): Map<string, Entry> {
  if (m.size <= MAX_AGENT_NAMES) return m;
  return new Map([...m.entries()].sort((a, b) => b[1].at - a[1].at).slice(0, MAX_AGENT_NAMES));
}

function load(kv: Pick<Storage, 'getItem'> | undefined = storage()): Names {
  if (current) return current;
  let raw: unknown;
  try { raw = kv?.getItem(KEY); } catch { raw = undefined; }
  current = parseAgentNames(raw);
  return current;
}

function save(next: Map<string, Entry>, kv: Pick<Storage, 'setItem' | 'removeItem'> | undefined) {
  current = trim(next);
  version++;
  try {
    if (current.size === 0) kv?.removeItem(KEY);
    else kv?.setItem(KEY, JSON.stringify({ v: 1, names: Object.fromEntries(current) }));
  } catch { /* holds for this visit */ }
  for (const l of listeners) l();
}

/** The name the person gave this agent, if any. */
export const agentNameOf = (id: string): string | undefined => load().get(id)?.name;

/** Names an agent; an empty or blank name gives it back its automatic one. */
export function setAgentName(id: string, raw: string, kv: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | undefined = storage(), now = Date.now()) {
  const name = cleanAgentName(raw);
  const next = new Map(load(kv));
  if (!name && !next.has(id)) return;
  if (name && next.get(id)?.name === name) return;
  if (name) { next.delete(id); next.set(id, { name, at: now }); } else next.delete(id);
  save(next, kv);
}

/** Forgets the names of agents that were removed from the list. */
export function forgetAgentNames(ids: readonly string[], kv: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | undefined = storage()) {
  const names = load(kv);
  if (!ids.some((id) => names.has(id))) return;
  const next = new Map(names);
  for (const id of ids) next.delete(id);
  save(next, kv);
}

const listeners = new Set<() => void>();
const subscribe = (cb: () => void) => { listeners.add(cb); return () => { listeners.delete(cb); }; };
/** A number that changes whenever a name does: a dependency for what is built from `tiles()`. */
export const useAgentNamesVersion = (): number => useSyncExternalStore(subscribe, () => { load(); return version; }, () => 0);

// For tests: forget what this visit held.
export const resetAgentNames = () => { current = undefined; version++; };

function storage(): Storage | undefined {
  try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; }
}

/** Gives tiles built without `tiles()` (the demo) the names the person chose. `_version` only ties a memo to the names. */
export function withOwnNames<T extends { id: string; name: string; autoName?: string }>(list: readonly T[], _version?: number): T[] {
  return list.map((a) => { const own = agentNameOf(a.id); return own ? { ...a, name: own, autoName: a.autoName ?? a.name } : a; });
}
