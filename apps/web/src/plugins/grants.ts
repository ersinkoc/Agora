// What the person granted each plugin (ADR-0114 decision 3, "Granting"; docs/ux/plugins.md, "Grants"). A grant is kept per plugin: the
// version it was given for, the capabilities the person was shown, and those they allowed. A capability not allowed answers "not
// granted". A version that asks for a capability the person was never shown is held (its panels are not offered) until they look again.
import { type Capability, isCapability } from './capabilities';
import type { PluginManifest } from './package';

export interface GrantRecord { readonly version: string; readonly seen: readonly Capability[]; readonly granted: readonly Capability[] }
export type GrantState = 'new' | 'held' | 'ready';

export function grantState(m: Pick<PluginManifest, 'capabilities'>, r: GrantRecord | undefined): GrantState {
  if (!r) return 'new';
  return m.capabilities.every((c) => r.seen.includes(c)) ? 'ready' : 'held';
}

/** The capabilities a plugin's frame is answered for: what this version asks for and the person allowed. Nothing for a held plugin. */
export function granted(m: Pick<PluginManifest, 'capabilities'>, r: GrantRecord | undefined): readonly Capability[] {
  if (grantState(m, r) !== 'ready' || !r) return [];
  return m.capabilities.filter((c) => r.granted.includes(c));
}

/** A new grant: the person saw everything this version asks for and allowed `allow` of it (anything else in `allow` is dropped). */
export function grant(m: Pick<PluginManifest, 'version' | 'capabilities'>, allow: readonly Capability[]): GrantRecord {
  return Object.freeze({ version: m.version, seen: Object.freeze([...m.capabilities]), granted: Object.freeze(m.capabilities.filter((c) => allow.includes(c))) });
}

/** Grants as this browser stored them; anything unreadable is left out (that plugin asks again). */
export function readGrants(v: unknown): Record<string, GrantRecord> {
  const out: Record<string, GrantRecord> = Object.create(null);
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return out;
  for (const [id, r] of Object.entries(v as Record<string, unknown>)) {
    if (typeof r !== 'object' || r === null) continue;
    const { version, seen, granted: g } = r as Record<string, unknown>;
    if (typeof version !== 'string' || !Array.isArray(seen) || !Array.isArray(g)) continue;
    const s = seen.filter(isCapability);
    out[id] = Object.freeze({ version, seen: Object.freeze(s), granted: Object.freeze(g.filter(isCapability).filter((c) => s.includes(c))) });
  }
  return out;
}
