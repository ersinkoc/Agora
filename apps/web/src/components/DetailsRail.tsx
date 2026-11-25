import { t } from '../i18n';
import { Icon } from './Icon';
import type { DetailsSection, DetailsTab } from './detailsTabs';

// The details panel's sections as a vertical menu at the window's end edge. It stays on screen when the
// panel is closed, so every section is one click away and any of them opens the panel.
export function DetailsRail({ sections, active, open, onSelect }: {
  sections: readonly DetailsSection[];
  active: DetailsTab;
  open: boolean;
  onSelect: (id: DetailsTab) => void;
}) {
  return <div role="tablist" aria-orientation="vertical" aria-label={t('ide-details')} className="ide-rail">
    {sections.map((s) => <button key={s.id} id={`details-tab-${s.id}`} type="button" role="tab" aria-selected={active === s.id}
      aria-controls={open ? 'details-panel' : undefined} title={s.label} onClick={() => onSelect(s.id)} className="ide-rail-item">
      <Icon name={s.icon} />
      <span className="sr-only">{s.label}</span>
      {!!s.badge && <span aria-hidden className={`ide-rail-badge ${s.id === 'needs' ? 'bg-needs-you text-on-accent' : 'bg-surface-2 text-ink'}`}>
        {s.badge > 99 ? '99+' : s.badge}</span>}
    </button>)}
  </div>;
}
