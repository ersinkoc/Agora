import type { SessionModel, SessionOptionModel } from '../model';

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

// "12.3k", "1.2M": a token count short enough for a chip.
export function shortCount(n: number, locale?: string): string {
  return new Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(n);
}

// The agent's own cost in its currency, or undefined when it reported none (or a currency the browser does not know).
export function ownCost(s: SessionModel | undefined, locale?: string): string | undefined {
  if (!s || s.costMicros === undefined || !s.currency) return undefined;
  try {
    return new Intl.NumberFormat(locale, { style: 'currency', currency: s.currency }).format(s.costMicros / 1_000_000);
  } catch {
    return undefined;
  }
}

// The current option first if the agent named one that is not in its list (it still is what runs), so the picker never claims another.
export function choices(list: readonly SessionOptionModel[], current: string): SessionOptionModel[] {
  if (!current || list.some((o) => o.id === current)) return [...list];
  return [{ id: current, name: current, description: '', skipsPermissions: false }, ...list];
}
