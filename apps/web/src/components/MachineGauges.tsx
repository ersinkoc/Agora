import type { CSSProperties } from 'react';
import { t } from '../i18n';
import { bytesLabel, diskLevel, type DiskAlert, type MachineFigures } from '../net/view';
import { Callout } from './kit';
import { ago } from '../net/time';

// ADR-0136: a machine's CPU, memory and every disk, as its node last reported them: a row of words and a bar each. A disk that runs low
// takes the needs-you colour, a critical one the danger colour, and says so in words too (colour is never the only sign). An offline
// machine's figures are its last ones, with when they were measured. Nothing when the machine reports none (an older node, or a platform
// without a reader yet).
export function MachineGauges({ figures, online, now }: { figures?: MachineFigures; online: boolean; now: number }) {
  if (!figures) return <p className="machine-gauges__none">{t('machine-figures-none')}</p>;
  const share = (part: number, whole: number) => (whole > 0 ? Math.min(100, Math.max(part > 0 ? 1 : 0, Math.round((part / whole) * 100))) : 0);
  const memUsed = Math.max(0, figures.memTotal - figures.memAvailable);
  const cpu = Math.round(figures.cpuBusy / 10);
  const bar = (percent: number, label: string, tone = '') => (
    <div className={`machine-meter__bar${tone ? ` machine-meter__bar--${tone}` : ''}`} role="img" aria-label={label}>
      <span style={{ '--share': `${percent}%` } as CSSProperties} />
    </div>
  );
  return (
    <div className="machine-gauges" data-machine-gauges>
      {figures.cpuCount > 0 && <div className="machine-meter">
        <div className="machine-meter__row"><span>{t('machine-cpu')}</span><span>{t('machine-cpu-value', { percent: cpu, count: figures.cpuCount })}</span></div>
        {bar(cpu, t('machine-cpu-label', { percent: cpu }))}
      </div>}
      {figures.memTotal > 0 && <div className="machine-meter">
        <div className="machine-meter__row"><span>{t('machine-ram')}</span><span>{t('machine-of', { used: bytesLabel(memUsed), total: bytesLabel(figures.memTotal) })}</span></div>
        {bar(share(memUsed, figures.memTotal), t('machine-ram-label', { percent: share(memUsed, figures.memTotal) }))}
      </div>}
      {figures.disks.map((d) => {
        const level = diskLevel(d);
        const used = share(d.total - d.free, d.total);
        return (
          <div key={d.mount} className="machine-meter" data-disk-level={level}>
            <div className="machine-meter__row">
              <span className="min-w-0 truncate"><bdi className="font-mono">{d.mount}</bdi>{d.workspaces && figures.workspaces > 0 ? ` · ${t('machine-disk-workspaces', { size: bytesLabel(figures.workspaces) })}` : ''}</span>
              <span className={level === 'critical' ? 'text-danger' : level === 'low' ? 'text-needs-you' : undefined}>
                {t(level === 'ok' ? 'machine-disk-free' : level === 'low' ? 'machine-disk-low' : 'machine-disk-critical', { free: bytesLabel(d.free) || '0 B', total: bytesLabel(d.total) })}
              </span>
            </div>
            {bar(used, t('machine-disk-label', { mount: d.mount, percent: used }), level === 'ok' ? '' : level)}
          </div>
        );
      })}
      {!online && figures.atMs > 0 && <p className="machine-gauges__when">{t('machine-figures-when', { when: ago(figures.atMs, now) })}</p>}
    </div>
  );
}

// ADR-0136: disks of online machines that run low, at the top of Home's "Needs you": the machine, the disk, what is free of its size,
// and where to look (the machine's card). Nothing when every disk has room. The node already cleans up the workspaces it made and
// refuses new ones below its minimum; this says what it cannot free on its own.
export function DiskAlertsBlock({ alerts }: { alerts: readonly DiskAlert[] }) {
  if (alerts.length === 0) return null;
  const critical = alerts.some((a) => a.level === 'critical');
  return (
    <div data-disk-alerts>
      <Callout tone={critical ? 'danger' : 'needs'} icon="warn" title={t('disk-alerts-title', { count: alerts.length })}>
        <ul className="m-0 grid list-none gap-1 p-0">
          {alerts.map((a) => (
            <li key={`${a.machine}\u0000${a.mount}`}>
              <a className="link" href={`#/machines/${encodeURIComponent(a.machine)}`}>{a.machineName}</a>
              {' · '}<bdi className="font-mono">{a.mount}</bdi>{' · '}
              <span className={a.level === 'critical' ? 'text-danger' : 'text-needs-you'}>{t('disk-alerts-free', { free: bytesLabel(a.free) || '0 B', total: bytesLabel(a.total) })}</span>
            </li>
          ))}
        </ul>
        <p className="m-0 mt-1 text-xs text-muted">{t('disk-alerts-hint')}</p>
      </Callout>
    </div>
  );
}
