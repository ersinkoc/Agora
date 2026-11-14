import { t } from '../i18n';

// ADR-0059: the agent was started with an option that lets it act without asking. Shown wherever the agent is (the tree, its tile
// and its pane header) for its whole life; the colour is the "needs you" token, the text says what it means.
export function NoAskBadge({ agent, compact }: { agent: { skipsPermissions?: boolean }; compact?: boolean }) {
  if (!agent.skipsPermissions) return null;
  return <span className={`ide-chip w-fit flex-none self-center justify-self-start border-needs-you/60 text-needs-you ${compact ? 'text-[0.625rem]' : ''}`} title={t('agent-does-not-ask-explain')}>{t('agent-does-not-ask')}</span>;
}
