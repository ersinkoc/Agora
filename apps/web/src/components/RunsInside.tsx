import type { AgentTileModel } from '../model';
import { t } from '../i18n';
import { visible } from '../net/view';
import { Icon } from './Icon';

/** ADR-0094: "Claude Code in Shell" while a person runs an agent by hand in this Shell. Found by the machine from the program in front of the
 * terminal, never from its screen; quiet chrome colours, never a state colour (the state pill says how it is). Its words are its name, so a
 * reader hears it with the row it is nested in. */
export function RunsInsideChip({ agent, nested }: { agent: Pick<AgentTileModel, 'runsInside' | 'closed'>; nested?: boolean }) {
  const inside = agent.closed ? undefined : agent.runsInside;
  if (!inside) return null;
  const words = t('agent-runs-inside', { name: visible(inside.name) });
  return (
    <span data-runs-inside className="ide-chip w-fit flex-none self-center justify-self-start" title={nested ? words : undefined}>
      <Icon name="terminal" className="size-3" />
      <span className="truncate">{words}</span>
    </span>
  );
}
