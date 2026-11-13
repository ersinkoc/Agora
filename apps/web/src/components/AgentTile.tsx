import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { StateBadge } from './StateBadge';

export function AgentTile({ agent, onOpen }: { agent: AgentTileModel; onOpen?: (id: string) => void }) {
  return (
    <button
      type="button"
      className={`tile tile--${agent.state}`}
      style={{ ['--tile-accent' as string]: agent.accent }}
      onClick={() => onOpen?.(agent.id)}
      aria-label={`${agent.name}, ${t(`state-${agent.state}`)}`}
    >
      <span className="tile__name">{agent.name}</span>
      <StateBadge state={agent.state} />
      <span className="muted">
        {t('agent-on-machine', { machine: agent.machine })} · {t('agent-workspace', { name: agent.workspace })}
      </span>
      {agent.parentName ? <span className="muted">{t('helper-started-by', { name: agent.parentName })}</span> : null}
      {agent.preview ? <span className="tile__preview" aria-hidden="true">{agent.preview}</span> : null}
    </button>
  );
}
