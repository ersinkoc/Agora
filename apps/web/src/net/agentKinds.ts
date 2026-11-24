// Settings → Agents: what a machine reports for each agent it can start (the `agentKinds` command), grouped and cleaned for display.
// Pure: no I/O but the optional last-known cache, which is read and written through the functions at the bottom.
import { usableOptions } from '../components/launchChoice';
import type { LaunchOptionChoice } from '../components/launchChoice';
import { usableSettings, type LaunchSettingChoice } from '../components/launchSettingsModel';
import { AGENT_COLOUR_NAMES } from '../themes/tokens';

// The wire's integration level (agora.v1.IntegrationLevel): 1 generic, 2 manifest, 3 ACP, 4 adapter process.
export interface KindInfo { kind: string; name: string; installed: boolean; level: number; program: string; settings: LaunchSettingChoice[]; options: LaunchOptionChoice[] }

/** How an agent runs, as the window says it. A chat agent has a structured conversation (ACP or an adapter); the rest run in a terminal. */
export type RunsAs = 'terminal' | 'chat' | 'adapter';
export const runsAs = (level: number): RunsAs => (level === 3 ? 'chat' : level === 4 ? 'adapter' : 'terminal');

/** Text that came from a machine or a manifest (a third party may have written it): plain, one line, no control or direction
 * characters, at most `max` characters. It is only ever rendered as text, never as markup. */
export function plainText(s: string, max = 48): string {
  // eslint-disable-next-line no-control-regex
  const one = s.replace(/[\u0000-\u001f\u007f-\u009f\u061c​-‏‪-‮⁦-⁩﻿]/g, ' ').replace(/\s+/g, ' ').trim();
  const chars = [...one];
  return chars.length > max ? `${chars.slice(0, max - 1).join('')}…` : one;
}

/** A kind's colour for its avatar: one of the agent colours, the same for a kind everywhere (Settings, and later the Start window). */
export function kindAccent(kind: string): string {
  let h = 0;
  for (const c of kind) h = (h * 31 + (c.codePointAt(0) ?? 0)) >>> 0;
  return `var(--agent-${AGENT_COLOUR_NAMES[h % AGENT_COLOUR_NAMES.length] ?? 'slate'})`;
}

/** ADR-0092: the prefix of every kind converted from the ACP registry (`agora_adapters::registry_convert`). Such a kind is untested in
 * Agora; a hand-made kind of the same name or id has already replaced it on the machine. */
export const REGISTRY_PREFIX = 'registry-';
export const isRegistryKind = (kind: string): boolean => kind.startsWith(REGISTRY_PREFIX);

export interface KindGroups { installed: KindInfo[]; missing: KindInfo[] }

/** Installed first group and not installed second, each with the hand-made kinds before the registry's (untested) ones, then by name;
 * the plain terminal ("shell") is not an agent and is left out. */
export function groupKinds(kinds: readonly KindInfo[]): KindGroups {
  const seen = new Set<string>();
  const rows = kinds.filter((k) => k.kind !== 'shell' && k.kind !== '' && !seen.has(k.kind) && (seen.add(k.kind), true));
  const byName = (a: KindInfo, b: KindInfo) => Number(isRegistryKind(a.kind)) - Number(isRegistryKind(b.kind))
    || plainText(a.name).localeCompare(plainText(b.name)) || a.kind.localeCompare(b.kind);
  return { installed: rows.filter((k) => k.installed).sort(byName), missing: rows.filter((k) => !k.installed).sort(byName) };
}

/** The settings a kind takes and the options it offers, cleaned. Options that skip permission questions come last and are flagged, so a row
 * can label them; none is ever a default. */
export function kindChoices(k: KindInfo): { settings: LaunchSettingChoice[]; options: LaunchOptionChoice[] } {
  const options = usableOptions(k.options);
  return { settings: usableSettings(k.settings), options: [...options.filter((o) => !o.skipsPermissions), ...options.filter((o) => o.skipsPermissions)] };
}

/** What one machine reported last, and when. `stale` when the machine is offline now (or the last ask failed). */
export interface KnownKinds { kinds: KindInfo[]; atMs: number }

const KEY = 'agora.agent-kinds.v1';
const MAX_KINDS = 160;

const isStr = (v: unknown): v is string => typeof v === 'string';
const cleanKind = (v: unknown): KindInfo | undefined => {
  if (typeof v !== 'object' || v === null) return undefined;
  const r = v as Record<string, unknown>;
  if (!isStr(r.kind) || !isStr(r.name)) return undefined;
  const list = <T,>(x: unknown, ok: (e: unknown) => e is T): T[] => (Array.isArray(x) ? x.filter(ok).slice(0, 32) : []);
  const obj = (e: unknown): e is Record<string, unknown> => typeof e === 'object' && e !== null;
  return {
    kind: r.kind.slice(0, 64), name: r.name.slice(0, 128), installed: r.installed === true, level: typeof r.level === 'number' ? r.level : 0, program: isStr(r.program) ? r.program.slice(0, 64) : '',
    settings: list(r.settings, obj).map((s) => ({ id: String(s.id ?? ''), label: String(s.label ?? ''), values: Array.isArray(s.values) ? s.values.filter(isStr).slice(0, 32) : [], defaultValue: String(s.defaultValue ?? '') })),
    options: list(r.options, obj).map((o) => ({ id: String(o.id ?? ''), label: String(o.label ?? ''), skipsPermissions: o.skipsPermissions === true })),
  };
};

/** The last-known list per machine from this browser's storage; anything malformed is dropped. */
export function readKnown(storage: Pick<Storage, 'getItem'> | undefined): Map<string, KnownKinds> {
  const out = new Map<string, KnownKinds>();
  try {
    const raw = storage?.getItem(KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    if (typeof parsed !== 'object' || parsed === null) return out;
    for (const [node, v] of Object.entries(parsed as Record<string, unknown>).slice(0, 64)) {
      const e = v as { kinds?: unknown; atMs?: unknown };
      if (!Array.isArray(e?.kinds) || typeof e.atMs !== 'number' || !Number.isFinite(e.atMs) || e.atMs < 0) continue;
      out.set(node, { atMs: e.atMs, kinds: e.kinds.slice(0, MAX_KINDS).map(cleanKind).filter((k): k is KindInfo => !!k) });
    }
  } catch { /* Storage can be unavailable or hold something else; the list then simply has no last-known entry. */ }
  return out;
}

export function writeKnown(storage: Pick<Storage, 'setItem'> | undefined, known: ReadonlyMap<string, KnownKinds>): void {
  try { storage?.setItem(KEY, JSON.stringify(Object.fromEntries(known))); } catch { /* Not saved: the list is still right for this visit. */ }
}
