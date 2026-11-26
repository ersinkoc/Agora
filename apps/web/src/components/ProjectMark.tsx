import { t } from '../i18n';
import type { ProjectSpread } from '../net/fleet';
import { visible } from '../net/view';
import { Icon } from './Icon';

// The same project open on several machines, said once and the same way everywhere (cards, the agent list, group headers): a chip with the
// machine count, the machines' names on hover. Only drawn for a project the hub identified on more than one machine (net/fleet.ts,
// `sharedAcross`); never from folder names.
export function ProjectChip({ spread, compact }: { spread: ProjectSpread; compact?: boolean }) {
  const count = spread.machines.length;
  const full = `${t('fleet-same-project', { count })}. ${t('fleet-same-project-on', { machines: spread.machines.map(visible).join(', ') })}`;
  return (
    <span className="ide-chip w-fit flex-none self-center text-xs" title={full} aria-label={full} role="img">
      <Icon name="folder" className="size-3" />
      <span aria-hidden>{compact ? `×${count}` : t('fleet-same-project', { count })}</span>
    </span>
  );
}

// A gentle hint: agents on more than one machine have changes in the same project right now. It does not say they conflict.
export function ProjectHint({ spread }: { spread: ProjectSpread }) {
  if (spread.changing < 2) return null;
  return <p role="note" className="m-0 flex items-center gap-1.5 text-xs text-muted"><Icon name="warn" className="size-3.5" />{t('fleet-project-changing', { count: spread.changing })}</p>;
}
