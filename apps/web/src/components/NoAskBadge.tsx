import { t } from '../i18n';
import { Icon } from './Icon';

// ADR-0059: the agent was started with an option that lets it act without asking. Shown wherever the agent is (the tree, its tile,
// its tab and its pane header) for its whole life; the colour is the "needs you" token.
// It is a compact badge: a warning sign and one short word ("Acts freely"), or the sign alone where there is no room (`icon`, a tab).
// The whole meaning ("Runs without asking: Started so that it acts without asking you first") is its name for screen readers and its
// tooltip; where the badge is not inside another control it can be focused (and so tapped on a touch screen) to show that text.
// `nested`: it sits inside a button or a tab, which must stay the one stop, so it is not focusable there and keeps title and name only.
export function NoAskBadge({ agent, compact, icon, nested }: { agent: { skipsPermissions?: boolean }; compact?: boolean; icon?: boolean; nested?: boolean }) {
  if (!agent.skipsPermissions) return null;
  const full = `${t('agent-does-not-ask')}: ${t('agent-does-not-ask-explain')}`;
  const tone = `tip ide-chip w-fit flex-none self-center justify-self-start border-needs-you/60 text-needs-you ${icon ? 'px-1' : ''} ${compact ? 'text-xs' : ''}`;
  const props = nested ? { title: full } : { tabIndex: 0, 'data-tip': full };
  return (
    <span role="img" aria-label={full} className={tone} {...props}>
      <Icon name="warn" className="size-3.5" />
      {!icon && <span aria-hidden>{t('agent-does-not-ask-short')}</span>}
    </span>
  );
}
