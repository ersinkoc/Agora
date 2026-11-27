import { useId } from 'react';
import { t } from '../i18n';
import type { modelPicks } from '../net/fleet';
import { visible } from '../net/view';
import { plain } from './boardColumns';
import { providerName } from './ModelChip';

// ADR-0123: narrow a list of agents (the fleet, the board's picker) to one provider or one model ("Every model" shows them all). The model is the agent's own word, kept short.
export function ModelPick({ value, picks, onChange }: { value?: string; picks: ReturnType<typeof modelPicks>; onChange: (v: string | undefined) => void }) {
  const id = useId();
  return (
    <span className="fleet-tools__pick">
      <label htmlFor={id} className="sr-only">{t('fleet-model-filter')}</label>
      <select id={id} value={value ?? ''} onChange={(e) => onChange(e.target.value || undefined)}>
        <option value="">{t('fleet-model-all')}</option>
        {picks.providers.length > 0 && <optgroup label={t('fleet-model-providers')}>
          {picks.providers.map((p) => <option key={p.value} value={p.value}>{t('fleet-model-count', { name: providerName(p.id), count: p.count })}</option>)}
        </optgroup>}
        {picks.models.length > 0 && <optgroup label={t('fleet-model-models')}>
          {picks.models.map((m) => <option key={m.value} value={m.value}>{t('fleet-model-count', { name: visible(plain(m.id, 48)), count: m.count })}</option>)}
        </optgroup>}
      </select>
    </span>
  );
}
