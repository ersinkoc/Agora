import { t } from '../i18n';
import { GROUP_BYS, type FleetGroup, type GroupBy } from '../net/fleet';

// "Group by" for the fleet console and the agent list: Machine, Project, Agent, State. Plain toggle buttons (one stop each, Enter or Space
// chooses), on one row at any width: the "Group by" word gives way first, then the buttons shrink (their title says the whole word), so every choice stays visible and reachable by keyboard and by touch.
export const GROUP_WORD: Record<GroupBy, string> = { machine: 'fleet-group-machine', project: 'fleet-group-project', kind: 'fleet-group-kind', state: 'fleet-group-state' };

export function GroupByControl({ by, onChange, label = true }: { by: GroupBy; onChange: (by: GroupBy) => void; label?: boolean }) {
  return (
    <div className="fleet-tools__by" role="group" aria-label={t('fleet-group-by')}>
      {label ? <span className="muted">{t('fleet-group-by')}</span> : null}
      {GROUP_BYS.map((g) => <button key={g} type="button" className="fleet-seg" title={t(GROUP_WORD[g])} aria-pressed={by === g} onClick={() => onChange(g)}>{t(GROUP_WORD[g])}</button>)}
    </div>
  );
}

// What a group is called: a state by the window's own word, a project or a kind by its name, and a nameless one by what it is not.
export const groupTitle = (g: FleetGroup, by: GroupBy): string => (g.labelKey ? t(g.labelKey) : g.name || t(by === 'kind' ? 'fleet-no-kind' : 'fleet-no-project'));
