import { t } from '../i18n';
import { Icon } from './Icon';

// ADR-0059: the agent was started with an option that lets it act without asking. Shown wherever the agent is (the tree, its tile,
// its tab and its pane header) for its whole life; the colour is the "needs you" token.
// It is a compact badge: a warning sign and one short word ("Acts freely"), or the sign alone where there is no room (`icon`, a tab).
// The whole meaning ("Runs without asking: Started so that it acts without asking you first") is its name for screen readers and its
// tooltip; where the badge is not inside another control it can be focused (and so tapped on a touch screen) to show that text.
// `nested`: it sits inside a button or a tab, which must stay the one stop, so it is not focusable there and keeps title and name only.
// `fit`: the sign alone until the nearest `@container` (an Agents-list row) is 16rem wide, then the word too, so a narrow row keeps the agent's name.
export function NoAskBadge({ agent, compact, icon, nested, session, approved, fit }: { agent: { skipsPermissions?: boolean }; compact?: boolean; icon?: boolean; nested?: boolean; session?: boolean; approved?: boolean; fit?: boolean }) {
  if (!agent.skipsPermissions) return null;
  // `session` (ADR-0090 Amendment A): it runs so after a switch and holds until it ends or is switched back. "With your approval" is said only when
  // this window saw that exact value approved (`approved`); after a restart, a resume or the agent's own switch it says only what is so.
  const full = `${t('agent-does-not-ask')}: ${t(session ? (approved ? 'agent-does-not-ask-session-explain' : 'agent-does-not-ask-session-neutral-explain') : 'agent-does-not-ask-explain')}`;
  const tone = `tip ide-chip w-fit flex-none self-center justify-self-start border-needs-you/60 text-needs-you ${icon ? 'px-1' : fit ? 'px-1 @3xs:px-2' : ''} ${compact ? 'text-xs' : ''}`;
  const props = nested ? { title: full } : { tabIndex: 0, 'data-tip': full };
  return (
    <span role="img" aria-label={full} className={tone} {...props}>
      <Icon name="warn" className="size-3.5" />
      {!icon && <span aria-hidden className={fit ? 'hidden @3xs:inline' : undefined}>{t('agent-does-not-ask-short')}</span>}
    </span>
  );
}
