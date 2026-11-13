// ADR-0059: what the start window offers from a kind's launch options, and the ids a start sends. "Ask before acting" (no option that
// skips permissions) is always the default; an option that skips permissions is never preselected and is chosen only after the user
// confirms it. Harmless options are plain on/off choices.
export interface LaunchOptionChoice { id: string; label: string; skipsPermissions: boolean }

/** The permission choice: `''` is "Ask before acting", otherwise the id of an option that skips permissions. */
export type PermissionChoice = string;
export const ASK: PermissionChoice = '';

export interface LaunchState {
  permission: PermissionChoice; // confirmed choice
  pending?: string; // an option that skips permissions, waiting for the confirmation
  harmless: string[]; // harmless options switched on
}
export const initialLaunch = (): LaunchState => ({ permission: ASK, harmless: [] });

export const skipping = (options: readonly LaunchOptionChoice[]) => options.filter((o) => o.skipsPermissions);
export const harmless = (options: readonly LaunchOptionChoice[]) => options.filter((o) => !o.skipsPermissions);

/** Picking a permission choice: "Ask" applies at once; an option that skips permissions waits for the confirmation. */
export function pickPermission(s: LaunchState, id: PermissionChoice): LaunchState {
  if (id === ASK) return { ...s, permission: ASK, pending: undefined };
  if (id === s.permission) return { ...s, pending: undefined };
  return { ...s, pending: id };
}
/** "Let the agent act without asking?": the primary answer keeps asking. */
export const confirmSkip = (s: LaunchState): LaunchState => (s.pending ? { ...s, permission: s.pending, pending: undefined } : s);
export const keepAsking = (s: LaunchState): LaunchState => ({ ...s, permission: ASK, pending: undefined });
export function toggleHarmless(s: LaunchState, id: string, on: boolean): LaunchState {
  const rest = s.harmless.filter((x) => x !== id);
  return { ...s, harmless: on ? [...rest, id] : rest };
}

/** An id this window may offer and sign: `[a-z0-9-]{1,32}`. The list comes through the hub and is only a display hint; an id with a
 * `!` or `,` (which a hub could use to pass for a confirmation or for two ids) is dropped before anything is shown or signed. */
export const usableId = (id: string) => /^[a-z0-9-]{1,32}$/.test(id);
export const usableOptions = (options: readonly LaunchOptionChoice[]) => options.filter((o) => usableId(o.id));

/** The ids a start sends, in the order the kind declares them; only options the kind offers, never a pending one. */
export function chosenIds(s: LaunchState, options: readonly LaunchOptionChoice[]): string[] {
  return usableOptions(options)
    .filter((o) => (o.skipsPermissions ? o.id === s.permission : s.harmless.includes(o.id)))
    .map((o) => o.id);
}

/** The ids whose "act without asking" confirmation THIS window showed and the user accepted: signed as their own list. The machine
 * decides from its own manifest which options skip permissions and refuses one that is not in this list. */
export function confirmedIds(s: LaunchState, options: readonly LaunchOptionChoice[]): string[] {
  return usableOptions(options).filter((o) => o.skipsPermissions && o.id === s.permission).map((o) => o.id);
}

/** A label is a Fluent id; one this window does not have shows the option's id instead of a bare key. */
export function labelOf(o: LaunchOptionChoice, translate: (id: string) => string): string {
  const said = o.label ? translate(o.label) : '';
  return said && said !== o.label ? said : o.id;
}
