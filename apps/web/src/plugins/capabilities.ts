// What a plugin may ask to see (ADR-0114 decision 3, rung 1). Each capability is read-only and scoped to what the person can already
// see in this window; the person grants them one by one (docs/ux/plugins.md). There are no write actions in rung 1: a capability that
// is not in this list is refused when a package names it, and answered "unknown capability" when a frame asks for it.
export const CAPABILITIES = ['agents.read', 'usage.read', 'tasks.read', 'changes.read', 'chat.read', 'theme.read', 'pane.context'] as const;
export type Capability = (typeof CAPABILITIES)[number];
export const isCapability = (v: unknown): v is Capability => typeof v === 'string' && (CAPABILITIES as readonly string[]).includes(v);

/** The Fluent key of a capability's plain words on the install screen (`plugin-cap-agents-read`, …). */
export const capabilityKey = (c: Capability) => `plugin-cap-${c.replace(/\./g, '-')}`;
