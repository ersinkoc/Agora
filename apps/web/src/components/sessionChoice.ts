import type { ConfigOptionModel, SessionModel, SessionOptionModel } from '../model';

// ADR-0058: what the session bar of a chat agent shows and asks. Pure, so it is tested without a page.

export type OptionKind = 'mode' | 'model';

// Choosing this needs the user to confirm first: a mode in which the agent acts without asking. Never chosen by default: the bar
// only ever shows what the agent reports as current and changes it on the user's own choice.
export function needsConfirm(kind: OptionKind, option: SessionOptionModel | undefined): boolean {
  return kind === 'mode' && !!option?.skipsPermissions;
}

// How full the context is, 0-100, or undefined when the agent did not say how big it is.
export function contextPercent(s: SessionModel | undefined): number | undefined {
  if (!s || s.contextWindow <= 0) return undefined;
  return Math.min(100, Math.round((s.contextTokens / s.contextWindow) * 100));
}

// The context fill in words: a level above 0 that rounds to 0% says "under 1%", never "0%" (an empty context would read as unused).
export function contextLabel(s: SessionModel | undefined): { key: 'session-context' | 'session-context-under-one'; percent: number } | undefined {
  const pct = contextPercent(s);
  if (pct === undefined || !s) return undefined;
  return pct === 0 && s.contextTokens > 0 ? { key: 'session-context-under-one', percent: 0 } : { key: 'session-context', percent: pct };
}

// The fill's design token: the accent while there is room, the needs-you color once the context is nearly full.
export const contextFill = (pct: number): 'bg-accent' | 'bg-needs-you' => (pct >= 80 ? 'bg-needs-you' : 'bg-accent');

// "12.3k", "1.2M": a token count short enough for a chip.
export function shortCount(n: number, locale?: string): string {
  return new Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(n);
}

// The current option first if the agent named one that is not in its list (it still is what runs), so the picker never claims another.
export function choices(list: readonly SessionOptionModel[], current: string): SessionOptionModel[] {
  if (!current || list.some((o) => o.id === current)) return [...list];
  return [{ id: current, name: current, description: '', skipsPermissions: false }, ...list];
}

// A confirmation waiting for the user, bound to the agent it was opened for: its hub-wide key `<node>/<agent>` (a local id alone is
// not unique: `n1/agent-1` and `n2/agent-1` are different agents).
export interface PendingChoice { agentKey: string; option: SessionOptionModel }

// The confirmation to show for the agent on screen now: none when it was opened for another one.
export const pendingFor = (p: PendingChoice | undefined, agentKey: string): PendingChoice | undefined => (p && p.agentKey === agentKey ? p : undefined);

// What "Switch anyway" sends: the mode to the agent the confirmation was opened for, and only while that agent is the one on screen.
export function confirmed(p: PendingChoice | undefined, onScreen: string): { agentKey: string; kind: 'mode'; id: string } | undefined {
  const bound = pendingFor(p, onScreen);
  return bound ? { agentKey: bound.agentKey, kind: 'mode', id: bound.option.id } : undefined;
}

// Where a hub-wide agent key points: the node (the producer that named it) and the agent's id there.
export function agentTarget(agentKey: string): { node: string; agentId: string } | undefined {
  const at = agentKey.indexOf('/');
  if (at <= 0 || at === agentKey.length - 1) return undefined;
  return { node: agentKey.slice(0, at), agentId: agentKey.slice(at + 1) };
}

// The picker's value: the reported current id, or the "not reported" placeholder (`''`), which is never one of the agent's options and
// sends nothing when picked. So nothing is shown as chosen that the agent did not report.
export const NOT_REPORTED = '';
export const needsPlaceholder = (current: string): boolean => current === NOT_REPORTED;

// Choosing what is already in use changes nothing: nothing is sent and no confirmation opens (a mode that acts without asking would
// otherwise ask again for the one it already runs).
export const isChange = (current: string, id: string): boolean => id !== NOT_REPORTED && id !== current;

// ---- ADR-0090: the agent's own settings (`AgentSessionConfig`): model, mode, reasoning effort and whatever else it lets the user change.

const FIRST: readonly ConfigOptionModel['category'][] = ['mode', 'model', 'thought_level'];

/** The settings to draw as chips in the composer (mode, model, effort, in that order) and the rest, for one "more" menu. A select with no
 *  choice cannot be set and is left out. `replaces`: the older pickers a setting stands in for (the same thing, set another way). */
export function configGroups(s: SessionModel | undefined): { chips: ConfigOptionModel[]; more: ConfigOptionModel[]; replaces: { mode: boolean; model: boolean } } {
  const all = (s?.config ?? []).filter((o) => o.type === 'boolean' || o.choices.length > 0);
  const chips = FIRST.flatMap((c) => all.filter((o) => o.category === c));
  return { chips, more: all.filter((o) => !FIRST.includes(o.category)), replaces: { mode: chips.some((o) => o.category === 'mode'), model: chips.some((o) => o.category === 'model') } };
}

/** What the window may do with a choice of a setting. A choice (or, for a boolean, the option as a whole) that the node marked as making
 *  the agent act without asking is `unavailable`: shown, disabled and NEVER sent, because the node cannot be told the user confirmed it. */
export type ChoiceState = 'current' | 'ok' | 'unavailable';
export function choiceState(o: ConfigOptionModel, value: string): ChoiceState {
  if (value === o.current) return 'current';
  const choice = o.choices.find((c) => c.id === value);
  return o.skipsPermissions || choice?.skipsPermissions ? 'unavailable' : 'ok';
}

/** The values a setting offers: a boolean's two, else its choices (the one in use first when the agent named it and does not list it). */
export function configChoices(o: ConfigOptionModel): SessionOptionModel[] {
  if (o.type === 'boolean') return ['true', 'false'].map((id) => ({ id, name: id, description: '', skipsPermissions: false }));
  return choices(o.choices, o.current);
}

/** Whether any choice of a shown setting is held back, so the window says why once. */
export const anyUnavailable = (list: readonly ConfigOptionModel[]): boolean => list.some((o) => configChoices(o).some((c) => choiceState(o, c.id) === 'unavailable'));
