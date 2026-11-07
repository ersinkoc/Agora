import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { StateBadge } from './StateBadge';

export function AttentionStrip({ agents, onOpen }: { agents: readonly AgentTileModel[]; onOpen?: (id: string) => void }) {
  return (
    <section className="strip" aria-label={t('attention-title')}>
      <span className="strip__title">{t('attention-title')}</span>
      {agents.length === 0 ? (
        <span className="muted">{t('attention-empty')}</span>
      ) : (
        agents.map((a) => (
          <button key={a.id} type="button" className="button" onClick={() => onOpen?.(a.id)}>
            {a.name} <StateBadge state={a.state} />
          </button>
        ))
      )}
    </section>
  );
}
