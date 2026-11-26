import type { AgentTileModel } from '../model';
import { t } from '../i18n';
import { visible } from '../net/view';
import { Icon } from './Icon';

/** ADR-0094: "Claude Code in Shell" while a person runs an agent by hand in this Shell. Found by the machine from the program in front of the
 * terminal, never from its screen; quiet chrome colours, never a state colour (the state pill says how it is). Its tooltip and accessible
 * name say how it was found and what it cannot see (ADR-0094 Amendment A); where it is not inside another control it can be focused (and so
 * tapped) to show that text, as the "Asks less" chip. */
export function RunsInsideChip({ agent, nested }: { agent: Pick<AgentTileModel, 'runsInside' | 'closed'>; nested?: boolean }) {
  const inside = agent.closed ? undefined : agent.runsInside;
  if (!inside) return null;
  const words = t('agent-runs-inside', { name: visible(inside.name) });
  const full = `${words}: ${t('agent-runs-inside-explain')}`;
  const props = nested ? { title: full } : { tabIndex: 0, 'data-tip': full };
  return (
    <span role="img" aria-label={full} data-runs-inside className="tip ide-chip w-fit flex-none self-center justify-self-start" {...props}>
      <Icon name="terminal" className="size-3" />
      <span aria-hidden className="truncate">{words}</span>
    </span>
  );
}
