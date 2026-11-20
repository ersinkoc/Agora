import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { ago } from '../net/time';
import { chipFor, quotaView, resetsText, resetAtText, startHint, toolName, windowLabel, type QuotaBar, type QuotaObs, type QuotaTool, CRITICAL_PERCENT } from '../net/quota';
import { agentKindOf } from '../net/fleet';
import { Icon } from './Icon';

// ADR-0091: the plan limits of subscription tools. One provider holds the observations and the ONE clock all of it reads (it ticks
// at most every 30 s; no row has a timer), so the Usage panel, a card chip and the start hint always agree. Without a provider there
// are no observations and nothing is drawn. No money anywhere.
const TICK_MS = 30_000;
interface QuotaValue { obs: readonly QuotaObs[]; machines: ReadonlyMap<string, string>; now: number }
const Ctx = createContext<QuotaValue>({ obs: [], machines: new Map(), now: 0 });

export function QuotaProvider({ obs, machines, children }: { obs: readonly QuotaObs[]; machines: ReadonlyMap<string, string>; children: ReactNode }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (obs.length === 0) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), TICK_MS);
    return () => clearInterval(id);
  }, [obs.length === 0]);
  const value = useMemo(() => ({ obs, machines, now }), [obs, machines, now]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
export const useQuota = (): QuotaValue => useContext(Ctx);

/** "Limit 82 %" on an agent's card or row, only from 80 percent and only for a fresh observation; amber, stronger from 95 percent. */
export function LimitChip({ agent, compact }: { agent: AgentTileModel; compact?: boolean }) {
  const { obs, now } = useQuota();
  const chip = useMemo(() => (obs.length ? chipFor(agent, obs, now) : undefined), [agent, obs, now]);
  if (!chip || agent.closed) return null; // a closed agent uses nothing more of the plan
  const full = `${t('quota-chip-title', { kind: toolName(agentKindOf(agent)), what: windowLabel(chip.window), percent: chip.percent })}${chip.resetsAtMs > now ? ` ${resetsText(chip.resetsAtMs, now)}` : ''}`;
  const tone = chip.level === 'critical' ? 'border-needs-you bg-needs-you/15 font-semibold text-needs-you' : 'border-needs-you/60 text-needs-you';
  return <span role="img" aria-label={full} title={full} className={`ide-chip w-fit flex-none self-center justify-self-start ${tone} ${compact ? 'text-xs' : ''}`} data-level={chip.level}>
    <Icon name="warn" className="size-3.5" />
    <span aria-hidden>{t('quota-chip', { percent: chip.percent })}</span>
  </span>;
}

/** A hint in the start window for the chosen kind on the chosen machine: from 95 percent, with the reset time. It never blocks the start. */
export function StartLimitHint({ tool, machineId }: { tool: string; machineId: string }) {
  const { obs, now } = useQuota();
  const hint = useMemo(() => (obs.length && tool ? startHint(tool, machineId, obs, now) : undefined), [tool, machineId, obs, now]);
  if (!hint || hint.percent < CRITICAL_PERCENT) return null;
  return <p role="note" className="m-0 text-xs text-needs-you" data-quota-hint>
    {t('quota-hint', { kind: toolName(tool), what: windowLabel(hint.window), percent: hint.percent })}
    {hint.resetsAtMs > now ? ` ${resetsText(hint.resetsAtMs, now)}` : ''}
  </p>;
}

function Bar({ b, now, showMachine }: { b: QuotaBar; now: number; showMachine: boolean }) {
  const where = showMachine ? `${b.machine}, ` : '';
  if (b.state === 'unknown') {
    return <li className="grid gap-0.5 py-1.5" data-state="unknown">
      <div className="flex min-w-0 items-baseline gap-2"><span className="min-w-0 flex-1 truncate text-muted">{t('quota-unknown')}</span></div>
      <p className="m-0 text-xs text-muted">{t('quota-unknown-why')}</p>
    </li>;
  }
  const label = windowLabel(b.window);
  const observed = t('quota-observed', { when: ago(now - b.ageMs, now) });
  if (b.state === 'reset') {
    return <li className="grid gap-0.5 py-1.5" data-state="reset">
      <div className="flex min-w-0 items-baseline gap-2"><span className="min-w-0 flex-1 truncate">{label}</span><span className="flex-none rounded-xs border border-line bg-surface-2 px-1.5 text-xs text-muted">{t('quota-reset')}</span></div>
      <p className="m-0 text-xs text-muted">{t('quota-reset-note', { when: resetAtText(b.resetsAtMs, now) })} · {observed}</p>
    </li>;
  }
  const percent = b.percent ?? 0;
  const stale = b.state === 'stale';
  const fill = stale ? 'bg-unknown' : b.level === 'normal' ? 'bg-accent' : 'bg-needs-you';
  const text = `${where}${toolName(b.tool)}, ${label}: ${t('quota-percent', { percent })}${stale ? `, ${t('quota-stale')}` : ''}`;
  return <li className="grid gap-1 py-1.5" data-state={b.state} data-level={b.level}>
    <div className="flex min-w-0 items-baseline gap-2">
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {stale && <span className="flex-none rounded-xs border border-line bg-surface-2 px-1.5 text-xs text-muted">{t('quota-stale')}</span>}
      {!stale && b.level !== 'normal' && <span className={`flex-none rounded-xs border px-1.5 text-xs text-needs-you ${b.level === 'critical' ? 'border-needs-you bg-needs-you/15 font-semibold' : 'border-needs-you/60'}`}>{t('quota-chip', { percent })}</span>}
      {(stale || b.level === 'normal') && <span className="flex-none text-xs text-muted">{t('quota-percent', { percent })}</span>}
    </div>
    <span role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} aria-valuetext={text} aria-label={`${where}${toolName(b.tool)}, ${label}`} className="relative block h-1.5 w-full overflow-hidden rounded-full border border-line bg-surface-2">
      <span className={`absolute inset-y-0 start-0 rounded-full ${fill}`} style={{ width: `${Math.max(percent, 2)}%` }} />
    </span>
    <p className="m-0 text-xs text-muted">{b.resetsAtMs > now ? resetsText(b.resetsAtMs, now) : t('quota-reset-unknown')} · {stale ? t('quota-stale-note') : observed}</p>
  </li>;
}

/** The "Plan limits" section of the Usage panel. */
export function QuotaSection({ fleet }: { fleet: readonly AgentTileModel[] }) {
  const { obs, machines, now } = useQuota();
  const tools: QuotaTool[] = useMemo(() => quotaView(obs, fleet, machines, now), [obs, fleet, machines, now]);
  return <section className="grid gap-1" aria-label={t('usage-limits-title')}>
    <h3 className="m-0 text-xs font-semibold tracking-wider text-muted uppercase">{t('usage-limits-title')}</h3>
    {tools.length === 0 ? <p className="muted m-0 rounded-s border border-dashed border-line px-3 py-2">{t('usage-limits-empty')}</p>
      : tools.map((tool) => {
        const several = tool.machines.length > 1 || machines.size > 1;
        return <div key={tool.tool} className="rounded-s border border-line bg-surface-0 px-3 py-1.5">
          <strong className="font-medium">{toolName(tool.tool)}</strong>
          {tool.machines.map((m) => <div key={m.machineId} className="grid">
            {several && <span className="mt-1 text-xs text-muted">{m.machine}</span>}
            <ul className="m-0 grid list-none divide-y divide-line p-0">{m.bars.map((b) => <Bar key={b.key} b={b} now={now} showMachine={several} />)}</ul>
          </div>)}
        </div>;
      })}
  </section>;
}
