import { useMemo } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { duration } from '../net/time';
import { usageView, type UsageAgent } from '../net/usageView';
import { shortCount } from './sessionChoice';
import { QuotaSection } from './Quota';
import { AgentAvatar, StatTile } from './kit';
import { EmptyState } from './EmptyState';

// The Usage panel of the rail: tokens, context and time per machine, project and agent. No money: most people pay a subscription, so a
// dollar figure says little, so no amount is ever shown here. Plan limits (ADR-0091) are what the tools themselves reported: fresh,
// stale, reset or unknown, never worked out.
const time = (ms?: number) => (ms === undefined ? '—' : duration(ms));

// A 6 px meter with full rounding. Amber only from 80 % (the quota warning family, as the plan limits); below it the accent.
function Meter({ pct, label }: { pct: number; label: string }) {
  return <span role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-label={label} className="relative h-1.5 w-20 flex-none overflow-hidden rounded-full border border-line bg-surface-2">
    <span className={`absolute inset-y-0 start-0 rounded-full ${pct >= 80 ? 'bg-needs-you' : 'bg-accent'}`} style={{ width: `${Math.max(pct, 2)}%` }} />
  </span>;
}

function AgentRow({ a }: { a: UsageAgent }) {
  return <li className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-0.5 py-1.5">
    <span className={`flex min-w-0 flex-1 basis-32 items-center gap-2 ${a.running ? 'text-ink' : 'text-muted'}`}><AgentAvatar accent={a.accent} kind={a.kind} size="s" /><span className="min-w-0 truncate">{a.name}</span></span>
    <span className="flex-none text-xs text-muted tabular-nums">{a.tokens > 0 ? shortCount(a.tokens) : '—'}</span>
    <span className="flex-none text-xs text-muted tabular-nums" title={t('usage-since-start')}>{time(a.activeMs)}</span>
    {a.contextPct !== undefined && <span className="flex flex-none items-center gap-1.5 text-xs text-muted"><Meter pct={a.contextPct} label={t('usage-context', { percent: a.contextPct })} />{a.contextPct}%</span>}
  </li>;
}

export function UsagePanel({ fleet, now }: { fleet: readonly AgentTileModel[]; now: number }) {
  const v = useMemo(() => usageView(fleet, now), [fleet, now]);
  if (fleet.length === 0 || (v.totalTokens === 0 && v.totalActiveMs === 0 && v.byProject.every((g) => g.agents.every((a) => a.contextPct === undefined)))) {
    return <div className="grid gap-4">
      <EmptyState icon="gauge" title={t('empty-usage-title')} hint={t('empty-usage-hint')} />
      <QuotaSection fleet={fleet} hideEmpty />
    </div>;
  }
  // The totals as the kit's stat tiles (none is tinted: nothing here asks for you), in one row that wraps when the panel is narrow; a number
  // never breaks over two lines.
  return <div className="grid gap-5">
    <div className="flex flex-wrap gap-2 [&>.kit-stat]:min-w-0 [&>.kit-stat]:flex-auto [&_.kit-stat\_\_n]:whitespace-nowrap" data-usage-totals>
      <StatTile value={shortCount(v.totalTokens)} label={t('usage-stat-tokens', { count: v.totalTokens })} />
      <StatTile value={time(v.totalActiveMs)} label={t('usage-since-start')} />
      <StatTile value={v.agents} label={t('usage-stat-agents', { count: v.agents })} />
    </div>
    {v.byMachine.length > 1 && <section className="grid gap-1.5" aria-label={t('usage-by-machine')}>
      <h3 className="panel-section-title">{t('usage-by-machine')}</h3>
      <ul className="m-0 grid list-none divide-y divide-line p-0">
        {v.byMachine.map((m) => <li key={m.id} className="flex items-center gap-3 py-1.5"><span className="min-w-0 flex-1 truncate">{m.machine}</span><span className="text-xs text-muted tabular-nums">{shortCount(m.tokens)}</span><span className="text-xs text-muted tabular-nums" title={t('usage-since-start')}>{time(m.activeMs)}</span></li>)}
      </ul>
    </section>}
    <section className="grid gap-2" aria-label={t('usage-by-project')}>
      <h3 className="panel-section-title">{t('usage-by-project')}</h3>
      {v.byProject.map((g) => <div key={g.key} className="rounded-m border border-line bg-surface-0 px-3 py-2">
        <div className="flex min-w-0 items-baseline gap-2"><strong className="min-w-0 flex-1 truncate font-medium">{g.project}</strong><span className="truncate text-xs text-muted">{g.machine}</span></div>
        <ul className="m-0 grid list-none divide-y divide-line p-0">{g.agents.map((a) => <AgentRow key={a.id} a={a} />)}</ul>
      </div>)}
    </section>
    <QuotaSection fleet={fleet} />
  </div>;
}
