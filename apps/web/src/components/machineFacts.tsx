import { t } from '../i18n';
import { visible } from '../net/hidden';
import { ago } from '../net/time';
import type { MachineModel } from '../net/view';
import { Icon, type IconName } from './Icon';
import { peerText, systemName } from './pairChoice';

const OS_ICON: Record<string, IconName> = { linux: 'osLinux', macos: 'osMac', windows: 'osWindows' };

// The system as a tile: a known one has its outline, anything else the plain machine. Decorative: the facts line names the system.
export function OsTile({ os }: { os: string }) {
  return <span aria-hidden className="os-tile"><Icon name={OS_ICON[os] ?? 'machine'} /></span>;
}

// A machine's name, and its id once beside it when the name does not already say it: two machines may share a host name, and the id
// (what the hub checked against the certificate) tells them apart wherever one is chosen or forgotten.
export function MachineName({ m, id, className = '' }: { m: Pick<MachineModel, 'id' | 'name'>; id?: string; className?: string }) {
  return (
    <span className="flex min-w-0 items-baseline gap-2">
      <strong id={id} className={`truncate ${className}`.trim()}>{visible(m.name)}</strong>
      {m.id !== m.name && <code dir="ltr" className="truncate text-xs font-normal text-muted">{visible(m.id)}</code>}
    </span>
  );
}

// "Linux · 0.3.35 · heard 4 seconds ago": what the machine says about itself (#944: the system as a person reads it, `os-*`; one this
// build has no words for as the machine reported it) and when the hub last heard it. Each part isolated, so none reorders another.
export function MachineFacts({ m, now }: { m: MachineModel; now: number }) {
  const when = t(m.online ? 'network-heard' : 'network-last-seen', { when: ago(m.lastSeenMs, now) });
  const parts: [string, boolean][] = [[peerText(systemName(m.os)) || '?', false], ...(m.build ? [[m.build, true] as [string, boolean]] : []), [when, false]];
  return <>{parts.map(([p, mono], i) => <span key={i}>{i > 0 && ' · '}<bdi data-fact={i === 0 ? 'os' : undefined} className={mono ? 'font-mono' : undefined}>{mono ? visible(p) : p}</bdi></span>)}</>;
}
