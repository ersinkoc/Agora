import { t } from '../i18n';
import { GROUP_BYS, type FleetGroup, type GroupBy } from '../net/fleet';
import { visible } from '../net/hidden';
import { SegmentedControl } from './kit';

// "Group by" for the fleet console and the agent list: Machine, Project, Agent, State. The kit's segmented control (a radio group named
// "Group by": one tab stop, the arrow keys move and choose), on one row at any width: the items shrink and their title says the whole word,
// so every choice stays visible and reachable by keyboard and by touch.
export const GROUP_WORD: Record<GroupBy, string> = { machine: 'fleet-group-machine', project: 'fleet-group-project', kind: 'fleet-group-kind', state: 'fleet-group-state' };

export function GroupByControl({ by, onChange }: { by: GroupBy; onChange: (by: GroupBy) => void }) {
  const items = GROUP_BYS.map((g) => ({ value: g, label: t(GROUP_WORD[g]), title: t(GROUP_WORD[g]) }));
  return <SegmentedControl label={t('fleet-group-by')} items={items} value={by} onChange={onChange} />;
}

// What a group is called: a state by the window's own word, a project or a kind by its name, and a nameless one by what it is not.
export const groupTitle = (g: FleetGroup, by: GroupBy): string => (g.labelKey ? t(g.labelKey) : visible(g.name) || t(by === 'kind' ? 'fleet-no-kind' : 'fleet-no-project'));
