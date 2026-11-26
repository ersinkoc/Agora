import { t } from '../i18n';
import { alwaysText, ALWAYS_LIST_MAX } from '../net/always';
import { approvalName } from '../net/approvalText';
import { plainText } from '../net/agentKinds';
import { actsFreelyNow } from '../net/give';
import { visible, type ApprovalModel } from '../net/view';
import type { AgentTileModel } from '../model';

// ADR-0101: what goes with "Allow always" on every card that offers it. The id ties the note to the button(s) it explains; one request
// can be on several surfaces of one page at once (its card, its banner, the attention dialog), so each surface has its own.
export type AlwaysSurface = 'card' | 'banner' | 'attention' | 'phone';
export const alwaysNoteId = (a: Pick<ApprovalModel, 'id'>, surface: AlwaysSurface): string => `always-note-${surface}-${a.id}`;

/** The button's own words: "Allow always" (with the words that say the agent keeps the answer: see `alwaysText`). */
export const allowAlwaysWord = (): string => t(alwaysText('approval-allow-always'));

/** Above the buttons of a request that offers only "always": the agent offers no single answer (`approval-always-only`). */
export function AlwaysOnly({ approval: x, className = '' }: { approval: ApprovalModel; className?: string }) {
  if (!x.always?.only) return null;
  return <p className={`m-0 text-sm text-ink ${className}`}>{t(alwaysText('approval-always-only'))}</p>;
}

/** Under the buttons: one muted line in the window's words, then the agent's own name for its option, quoted and shown as text only (it
 * is the agent's, untrusted: never a control's name). */
export function AlwaysNote({ approval: x, surface, className = '' }: { approval: ApprovalModel; surface: AlwaysSurface; className?: string }) {
  if (!x.always) return null;
  const label = x.always.label.trim();
  return (
    <div id={alwaysNoteId(x, surface)} className={`grid gap-0.5 text-xs text-muted ${className}`}>
      <p className="m-0">{t(alwaysText('approval-allow-always-note'), { name: visible(approvalName(x)) })}</p>
      {label && <p className="m-0 break-words" dir="auto">{t('approval-allow-always-label', { label: visible(plainText(label, 120)) })}</p>}
    </div>
  );
}

/** ADR-0101: "Asks less", while the agent holds at least one "always" answer. Quiet chrome colours, never a state colour, and never next to
 * "Acts freely" (that one says more). Its full meaning, with the requests it was allowed (their actions, bounded), is its name and tooltip;
 * where it is not inside another control it can be focused (and so tapped) to show that text, as the "Acts freely" badge. */
export function AsksLessChip({ agent, compact, nested }: { agent: Pick<AgentTileModel, 'alwaysAllowed' | 'skipsPermissions' | 'session'>; compact?: boolean; nested?: boolean }) {
  const held = agent.alwaysAllowed ?? [];
  if (held.length === 0 || actsFreelyNow(agent as AgentTileModel)) return null;
  const actions = held.slice(-ALWAYS_LIST_MAX).map((a) => visible(plainText(a, 48))).filter(Boolean);
  const full = `${t('agent-always-allowed-short')}: ${t(alwaysText('agent-always-allowed-explain'), { count: held.length })}${actions.length ? ` ${actions.join(' · ')}` : ''}`;
  const props = nested ? { title: full } : { tabIndex: 0, 'data-tip': full };
  return (
    <span role="img" aria-label={full} data-asks-less className={`tip ide-chip w-fit flex-none self-center justify-self-start ${compact ? 'text-xs' : ''}`} {...props}>
      <span aria-hidden>{t('agent-always-allowed-short')}</span>
    </span>
  );
}
