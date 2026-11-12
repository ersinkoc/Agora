import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { getLanguage, t } from '../i18n';
import { machineKey, type AgentTileModel } from '../model';
import { money, type UsageSummary } from '../net/usage';
import { bytesLabel, type MachineModel } from '../net/view';
import { Icon } from './Icon';
import { StateMark } from './StateBadge';

const ROW = 104;

function ago(ms: number, now: number): string {
  if (!ms) return t('network-never');
  const s = Math.max(0, Math.round((now - ms) / 1000));
  const rtf = new Intl.RelativeTimeFormat(getLanguage(), { numeric: 'auto' });
  if (s < 60) return rtf.format(-s, 'second');
  if (s < 3600) return rtf.format(-Math.round(s / 60), 'minute');
  if (s < 86400) return rtf.format(-Math.round(s / 3600), 'hour');
  return rtf.format(-Math.round(s / 86400), 'day');
}

// The hub in the middle, every machine it knows around it: who is connected right now, what runs there, when it was last heard.
export function NetworkView({ machines, agents, usage, hubName }: { machines: readonly MachineModel[]; agents: readonly AgentTileModel[]; usage?: UsageSummary; hubName: string }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const id = setInterval(() => setNow(Date.now()), 5000); return () => clearInterval(id); }, []);
  const online = machines.filter(m => m.online).length;
  // The lines end at the middle of each card as it really is laid out (cards grow with the agents listed in them).
  const list = useRef<HTMLUListElement>(null);
  const [ys, setYs] = useState<number[]>([]);
  const [height, setHeight] = useState(ROW);
  useLayoutEffect(() => {
    const el = list.current;
    if (!el) return;
    const measure = () => {
      const top = el.getBoundingClientRect().top;
      setYs([...el.children].map((c) => { const r = c.getBoundingClientRect(); return r.top - top + r.height / 2; }));
      setHeight(el.offsetHeight);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    el.childNodes.forEach((c) => ro.observe(c as Element));
    return () => ro.disconnect();
  }, [machines.length, agents.length]);
  const mid = height / 2;
  const totalMem = machines.reduce((n, m) => n + (m.memoryBytes ?? 0), 0);

  return <div className="grid gap-4 md:grid-cols-[14rem_5rem_minmax(0,1fr)]">
    <section aria-label={t('network-hub')} className="grid content-center md:min-h-[var(--net-h)]" style={{ ['--net-h' as string]: `${height}px` }}>
      <div className="grid gap-2 rounded-m border border-accent bg-surface-1 p-4 shadow-card">
        <div className="flex items-center gap-2"><span aria-hidden className="grid size-8 place-items-center rounded-xs bg-accent text-sm font-bold text-on-accent">A</span><h2 className="m-0 text-sm normal-case tracking-normal text-ink">{t('network-hub')}</h2></div>
        <code dir="ltr" className="break-all text-xs text-muted">{hubName}</code>
        <p className="m-0 flex items-center gap-1.5"><span aria-hidden className={`size-2 rounded-full ${online ? 'bg-finished' : 'bg-needs-you'}`} />{t('network-online', { online, total: machines.length })}</p>
        <p className="muted m-0">{t('ide-status-agents', { count: agents.length })}{totalMem > 0 ? ` · ${bytesLabel(totalMem)}` : ''}</p>
        {usage && usage.totalMicroUsd > 0 && <p className="muted m-0">{money(usage.totalMicroUsd)}</p>}
      </div>
    </section>

    <svg aria-hidden className="hidden md:block" width="80" height={height} viewBox={`0 0 80 ${height}`}>
      {machines.map((m, i) => {
        const y = ys[i] ?? i * 116 + ROW / 2;
        return <g key={m.id}>
          <path d={`M0 ${mid} C 40 ${mid}, 40 ${y}, 80 ${y}`} fill="none" strokeWidth="2" className={m.online ? 'net-link net-link--on' : 'net-link'} stroke={m.online ? 'var(--state-finished)' : 'var(--state-unknown)'} strokeDasharray={m.online ? '6 6' : '2 6'} />
          <circle cx="80" cy={y} r="3.5" fill={m.online ? 'var(--state-finished)' : 'var(--state-unknown)'} />
        </g>;
      })}
    </svg>

    <ul ref={list} className="m-0 grid list-none gap-3 p-0">
      {machines.length === 0 && <li className="muted">{t('machines-empty')}</li>}
      {machines.map(m => {
        const mine = agents.filter(a => machineKey(a) === m.id);
        return <li key={m.id} className={`grid gap-2 rounded-m border bg-surface-1 p-3 ${m.online ? 'border-line' : 'border-dashed border-line opacity-80'}`} style={{ minHeight: ROW }}>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <Icon name="machine" className="size-4 text-muted" />
            <strong className="min-w-0 truncate">{m.name}</strong>
            <span className={`ide-chip ${m.online ? 'text-finished' : ''}`}><span aria-hidden className={`size-1.5 rounded-full ${m.online ? 'bg-finished' : 'bg-unknown'}`} />{t(m.online ? 'machine-online' : 'machine-offline')}</span>
            <span className="ide-chip">{m.os || '?'}</span>
            {m.build && <span className="ide-chip font-mono" title={t('network-build')}>{m.build}</span>}
            <span className="flex-1" />
            <span className="text-xs text-muted">{m.online ? t('network-heard', { when: ago(m.lastSeenMs, now) }) : t('network-last-seen', { when: ago(m.lastSeenMs, now) })}</span>
          </div>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted">
            <code dir="ltr" className="break-all">{m.id}</code>
            <span>{t('machine-agents', { count: m.agentsRunning })}</span>
            {!!m.terminalsOpen && <span>{t('network-terminals', { count: m.terminalsOpen })}</span>}
            {!!m.memoryBytes && <span className="font-mono">{bytesLabel(m.memoryBytes)}</span>}
            {usage?.perNode.has(m.id) && <span>{t('machine-cost', { amount: money(usage.perNode.get(m.id) ?? 0) })}</span>}
          </div>
          {mine.length > 0 && <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0">
            {mine.map(a => <li key={a.id}><a href={`#/agent/${encodeURIComponent(a.id)}`} className="ide-chip hover:text-ink"><StateMark state={a.state} />{a.name}</a></li>)}
          </ul>}
        </li>;
      })}
    </ul>
  </div>;
}
