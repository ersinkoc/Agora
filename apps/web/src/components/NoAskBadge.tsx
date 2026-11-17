import { t } from '../i18n';
import { Icon } from './Icon';

// ADR-0059: the agent was started with an option that lets it act without asking. Shown wherever the agent is (the tree, its tile
// and its pane header) for its whole life; the colour is the "needs you" token, the text says what it means.
// `icon`: where there is no room for the words (a tab, a pane header) it is a shield with a line through it, and the words are its name
// for screen readers and its tooltip. It never shows the words cut short: a safety signal says all of it or is a plain symbol.
export function NoAskBadge({ agent, compact, icon }: { agent: { skipsPermissions?: boolean }; compact?: boolean; icon?: boolean }) {
  if (!agent.skipsPermissions) return null;
  const words = t('agent-does-not-ask');
  const tone = 'ide-chip w-fit self-center justify-self-start border-needs-you/60 text-needs-you';
  if (icon) return <span className={`${tone} flex-none px-1`} title={`${words}: ${t('agent-does-not-ask-explain')}`}><Icon name="shieldOff" className="size-3.5" /><span className="sr-only">{words}</span></span>;
  return <span className={`${tone} flex-none ${compact ? 'text-xs' : ''}`} title={t('agent-does-not-ask-explain')}>{words}</span>;
}
