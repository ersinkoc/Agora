import { useMemo } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { duration } from '../net/time';
import { usageView, type UsageAgent } from '../net/usageView';
import { shortCount } from './sessionChoice';

// The Usage panel of the rail: tokens, context and time per machine, project and agent. No money: most people pay a subscription, so a
// dollar figure says little. What an agent itself reports about cost sits behind a disclosure; plan limits have a place and an honest
// empty state until something reports them.
const time = (ms?: number) => (ms === undefined ? '—' : duration(ms));

function Meter({ pct, label }: { pct: number; label: string }) {
  return <span role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-label={label} className="relative h-1.5 w-14 flex-none overflow-hidden rounded-full border border-line bg-surface-2">
    <span className={`absolute inset-y-0 start-0 rounded-full ${pct >= 80 ? 'bg-needs-you' : 'bg-accent'}`} style={{ width: `${Math.max(pct, 2)}%` }} />
  </span>;
}

function AgentRow({ a }: { a: UsageAgent }) {
  return <li className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-0.5 py-1.5">
    <span className={`min-w-0 flex-1 basis-32 truncate ${a.running ? 'text-ink' : 'text-muted'}`}>{a.name}</span>
    <span className="flex-none text-xs text-muted">{shortCount(a.tokens)}</span>
    <span className="flex-none text-xs text-muted">{time(a.activeMs)}</span>
    {a.contextPct !== undefined && <span className="flex flex-none items-center gap-1.5 text-xs text-muted"><Meter pct={a.contextPct} label={t('usage-context', { percent: a.contextPct })} />{a.contextPct}%</span>}
  </li>;
}

export function UsagePanel({ fleet, now }: { fleet: readonly AgentTileModel[]; now: number }) {
  const v = useMemo(() => usageView(fleet, now), [fleet, now]);
  if (fleet.length === 0 || (v.totalTokens === 0 && v.totalActiveMs === 0 && v.byProject.every((g) => g.agents.every((a) => a.contextPct === undefined)))) {
    return <div className="grid gap-4">
      <p className="muted m-0">{t('usage-empty')}</p>
      <Limits />
    </div>;
  }
  return <div className="grid gap-4">
    <div className="grid gap-0.5">
      <p className="m-0 text-2xl font-semibold tracking-tight">{t('usage-total-tokens', { count: shortCount(v.totalTokens) })}</p>
      <p className="m-0 text-xs text-muted">{t('usage-total-time', { time: time(v.totalActiveMs) })} · {t('usage-agents', { count: v.agents })}</p>
    </div>
    {v.byMachine.length > 1 && <section className="grid gap-1" aria-label={t('usage-by-machine')}>
      <h3 className="m-0 text-xs font-semibold tracking-wider text-muted uppercase">{t('usage-by-machine')}</h3>
      <ul className="m-0 grid list-none divide-y divide-line p-0">
        {v.byMachine.map((m) => <li key={m.machine} className="flex items-center gap-3 py-1.5"><span className="min-w-0 flex-1 truncate">{m.machine}</span><span className="text-xs text-muted">{shortCount(m.tokens)}</span><span className="text-xs text-muted">{time(m.activeMs)}</span></li>)}
      </ul>
    </section>}
    <section className="grid gap-2" aria-label={t('usage-by-project')}>
      <h3 className="m-0 text-xs font-semibold tracking-wider text-muted uppercase">{t('usage-by-project')}</h3>
      {v.byProject.map((g) => <div key={g.key} className="rounded-s border border-line bg-surface-0 px-3 py-1.5">
        <div className="flex min-w-0 items-baseline gap-2"><strong className="min-w-0 flex-1 truncate font-medium">{g.project}</strong><span className="truncate text-xs text-muted">{g.machine}</span></div>
        <ul className="m-0 grid list-none divide-y divide-line p-0">{g.agents.map((a) => <AgentRow key={a.id} a={a} />)}</ul>
      </div>)}
    </section>
    <Limits />
    {v.reported.length > 0 && <details className="group rounded-s border border-line bg-surface-0">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-2 px-3 py-2 text-xs text-muted hover:text-ink [&::-webkit-details-marker]:hidden">{t('usage-reported-toggle')}</summary>
      <div className="grid gap-1 border-t border-line px-3 py-2 text-xs">
        <p className="m-0 text-muted">{t('usage-reported-note')}</p>
        <ul className="m-0 grid list-none gap-0.5 p-0">{v.reported.map((r) => <li key={r.id}>{t('usage-reported-row', { name: r.name, amount: r.amount })}</li>)}</ul>
      </div>
    </details>}
  </div>;
}

// Plan quota windows: a research task is scoping what can be shown. Nothing here is invented; it says so until an agent reports limits.
function Limits() {
  return <section className="grid gap-1" aria-label={t('usage-limits-title')}>
    <h3 className="m-0 text-xs font-semibold tracking-wider text-muted uppercase">{t('usage-limits-title')}</h3>
    <p className="muted m-0 rounded-s border border-dashed border-line px-3 py-2">{t('usage-limits-empty')}</p>
  </section>;
}
