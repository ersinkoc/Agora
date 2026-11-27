import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { visible } from '../net/view';
import { plain } from './boardColumns';

// ADR-0123: the model an agent runs on and its reasoning effort ("opus · high"), beside its other chips in its pane header. Both are the
// agent's own words (or the launch settings it was started with), so they are shown as one short plain line; the whole of both is the
// tooltip. Nothing when neither is known.
export function ModelChip({ agent }: { agent: Pick<AgentTileModel, 'model' | 'effort'> }) {
  const model = visible(plain(agent.model ?? '', 48));
  const effort = visible(plain(agent.effort ?? '', 24));
  if (!model && !effort) return null;
  const full = [model && t('agent-model-title', { value: model }), effort && t('agent-effort-title', { value: effort })].filter(Boolean).join(' · ');
  const short = model ? `${model}${effort ? ` · ${effort}` : ''}` : t('agent-effort-chip', { value: effort });
  return (
    <span role="img" aria-label={full} data-model-chip className="tip ide-chip w-fit max-w-[24ch] flex-none self-center justify-self-start" tabIndex={0} data-tip={full}>
      <span aria-hidden className="truncate">{short}</span>
    </span>
  );
}
