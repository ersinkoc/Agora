import { useMemo, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { type AttentionItem, type FleetFilter, type GroupBy, FLEET_FILTERS, type FleetCounts, collapseAttention, decidableInline, emptyKind, fleetView, groupFleet } from '../net/fleet';
import { loadGroupBy, saveGroupBy } from '../net/fleetPrefs';
import { ago } from '../net/time';
import { money, type UsageSummary } from '../net/usage';
import { approvalName } from '../net/approvalText';
import { detailText } from '../net/tool';
import { type ApprovalModel, type MachineModel, visible } from '../net/view';
import { AgentTile } from './AgentTile';
import { Icon } from './Icon';
import { StateBadge, StateMark } from './StateBadge';
import type { StartChoice } from './StartApproval';

export const ATTENTION_SHOWN = 6;
export type LinkState = 'connecting' | 'live' | 'offline';

// What one filter chip says, in the state words the rest of the window uses.
const FILTER_WORD: Record<FleetFilter, string> = { 'needs-you': 'state-needs-you', working: 'state-working', finished: 'state-finished', failed: 'fleet-failed' };
const countOf = (c: FleetCounts, f: FleetFilter): number => (f === 'needs-you' ? c.needs : c[f]);

// The header: five answers, each a live count. Four are filters of the tiles below (a press shows only those, a second press shows
// everything again); the machines count opens the machines page. A count of zero is quiet and not pressable, unless it is the filter
// in use (then it can still be turned off).
export function FleetSummary({ counts, filter, onFilter }: { counts: FleetCounts; filter?: FleetFilter; onFilter?: (f: FleetFilter | undefined) => void }) {
  const allOnline = counts.machinesTotal > 0 && counts.machinesOnline === counts.machinesTotal;
  return (
    <div className="fleet-summary" role="group" aria-label={t('fleet-summary-label')}>
      {FLEET_FILTERS.map((f) => {
        const n = countOf(counts, f);
        const on = filter === f;
        return (
          <button key={f} type="button" className={`fleet-count fleet-count--${f}${n === 0 && !on ? ' fleet-count--zero' : ''}`} aria-pressed={on} disabled={!onFilter || (n === 0 && !on)}
            onClick={() => onFilter?.(on ? undefined : f)} title={`${t('fleet-filter-hint')}: ${t(FILTER_WORD[f])}`}>
            <span className="fleet-count__n">{n}</span>
            <span className="fleet-count__word"><StateBadge state={f === 'failed' ? 'finished' : f} failed={f === 'failed'} word={t(FILTER_WORD[f])} /></span>
          </button>
        );
      })}
      <a className="fleet-count fleet-count--machines" href="#/machines">
        <span aria-hidden className={`fleet-dot ${allOnline ? 'bg-finished' : counts.machinesTotal === 0 ? 'bg-unknown' : 'bg-needs-you'}`} />
        <span className="fleet-count__word">{counts.machinesTotal === 0 ? t('machines-online', { count: 0 }) : t('fleet-machines', { online: counts.machinesOnline, total: counts.machinesTotal })}</span>
      </a>
    </div>
  );
}

// The one line of what an agent asks: the request's own words, short. Anything long goes through the full card.
function askText(x: ApprovalModel): string {
  if (x.start) return t('fleet-asks-start');
  const d = x.detail ? detailText(x.detail) : '';
  return visible(d ? `${x.action} · ${d}` : x.action);
}

const askOf = (it: AttentionItem): string => (it.kind === 'approval' ? askText(it.approval) : t('fleet-waiting-in-terminal'));

// The queue: everything that needs the user, most urgent first. A plain yes or no has Allow and Deny in its row (the same answer, signed
// the same way, as the banner above the terminal); anything with something to read opens the agent, where the whole request is.
export function AttentionQueue({ items, machines, now, onOpen, onResolve, onReview, stale }: {
  items: readonly AttentionItem[];
  machines: readonly MachineModel[];
  now: number;
  onOpen: (agentId: string) => void;
  onResolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice) => void;
  // Where a request is read in full when this window does not list its agent (so there is no agent to open).
  onReview?: () => void;
  // The link is not live: an answer cannot be signed to a machine through it, so Allow and Deny wait, with the reason shown.
  stale?: boolean;
}) {
  const machineName = (id: string) => machines.find((m) => m.id === id)?.name;
  // A glance is a few rows: the most urgent ones, and the rest one press away. Many identical asks are one row.
  const [all, setAll] = useState(false);
  const [each, setEach] = useState<ReadonlySet<string>>(new Set());
  const rows = useMemo(() => collapseAttention(items), [items]);
  const shown = all ? rows : rows.slice(0, ATTENTION_SHOWN);
  // How long it has waited is shown on every row or on none: a list where some rows say it and others do not reads as if those were fresh.
  const knownWait = items.length > 0 && items.every((x) => !!x.sinceMs);
  const toggleEach = (key: string) => setEach((cur) => { const next = new Set(cur); if (!next.delete(key)) next.add(key); return next; });

  const rowOf = (it: AttentionItem, inlineOk: boolean) => {
    const agent = it.agent;
    const name = it.kind === 'approval' ? (agent?.name || approvalName(it.approval)) : it.agent.name;
    const machine = agent?.machine || (it.kind === 'approval' ? it.approval.machineName || machineName(it.approval.node) : undefined);
    const project = agent ? agent.project || agent.workspace : undefined;
    const inline = inlineOk && it.kind === 'approval' && !!onResolve && decidableInline(it.approval);
    const ask = askOf(it);
    const why = `fleet-stale-why-${it.key}`;
    return (
      <li key={it.key} className="fleet-ask">
        <div className="fleet-ask__who">
          <StateMark state="needs-you" />
          <strong className="fleet-ask__name">{name}</strong>
          <span className="fleet-ask__where">{[machine, project].filter(Boolean).join(' · ')}</span>
          {knownWait && it.sinceMs ? <time className="fleet-ask__time" dateTime={new Date(it.sinceMs).toISOString()}>{t('fleet-asked', { when: ago(it.sinceMs, now) })}</time> : null}
        </div>
        {/* A request that can be answered here is shown whole, never cut; the others may be cut because they are read in the full card. */}
        <p className={`fleet-ask__what${inline ? '' : ' fleet-ask__what--clamp'}`} title={ask}>{ask}</p>
        <div className="fleet-ask__actions">
          {inline && it.kind === 'approval' ? <>
            <button type="button" className="button button--primary" disabled={stale} aria-describedby={stale ? why : undefined} aria-label={t('fleet-allow-for', { name })} onClick={() => onResolve?.(it.approval, true)}>{t('approval-allow')}</button>
            <button type="button" className="button" disabled={stale} aria-describedby={stale ? why : undefined} aria-label={t('fleet-deny-for', { name })} onClick={() => onResolve?.(it.approval, false)}>{t('approval-deny')}</button>
            {stale && <span id={why} className="fleet-ask__why">{t('fleet-stale-no-answer')}</span>}
          </> : agent ? <button type="button" className="button" aria-label={t(it.kind === 'approval' ? 'fleet-review-for' : 'fleet-open-for', { name })} onClick={() => onOpen(agent.id)}>{t(it.kind === 'approval' ? 'fleet-review' : 'action-open')}</button>
            : onReview ? <button type="button" className="button" aria-label={t('fleet-review-for', { name })} onClick={onReview}>{t('fleet-review')}</button> : null}
        </div>
      </li>
    );
  };

  return (
    <section className="fleet-attention" aria-labelledby="fleet-attention-h">
      <div className="fleet-section__head">
        <h2 id="fleet-attention-h" className="fleet-section__title text-needs-you">{t('attention-title')}{items.length > 0 && <span className="fleet-section__n"> · {items.length}</span>}</h2>
        {items.length > 0 && <span className="fleet-hint"><kbd className="ide-kbd">N</kbd> {t('ide-welcome-next')}</span>}
      </div>
      {items.length === 0 ? <p className="fleet-calm muted m-0">{t('attention-empty')}</p> : (
        <ul className="fleet-queue">
          {shown.map((row) => row.kind === 'one' ? rowOf(row.item, true) : (
            <li key={row.key} className="fleet-ask fleet-ask--many">
              <div className="fleet-ask__who">
                <StateMark state="needs-you" />
                <strong className="fleet-ask__name">{row.agentKind ? t('fleet-same-ask', { count: row.items.length, kind: row.agentKind, machine: row.machine }) : t('fleet-same-ask-any', { count: row.items.length, machine: row.machine })}</strong>
              </div>
              <p className="fleet-ask__what fleet-ask__what--clamp" title={askOf(row.items[0] as AttentionItem)}>{askOf(row.items[0] as AttentionItem)}</p>
              <div className="fleet-ask__actions">
                <button type="button" className="button" aria-expanded={each.has(row.key)} onClick={() => toggleEach(row.key)}>{each.has(row.key) ? t('fleet-collapse') : t('fleet-review-each')}</button>
              </div>
              {each.has(row.key) && <ul className="fleet-queue fleet-ask__each">{row.items.map((it) => rowOf(it, false))}</ul>}
            </li>
          ))}
        </ul>
      )}
      {rows.length > ATTENTION_SHOWN && <button type="button" className="button fleet-more" aria-expanded={all} onClick={() => setAll((v) => !v)}>{all ? t('fleet-attention-less') : t('fleet-attention-more', { count: rows.length - ATTENTION_SHOWN })}</button>}
    </section>
  );
}

// A command to copy: shown as it is typed, never translated.
function Command({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  const copy = () => { try { void navigator.clipboard?.writeText(text).then(() => setDone(true), () => undefined); } catch { /* the text can be selected and copied by hand */ } };
  return (
    <div className="fleet-command">
      <code dir="ltr" className="fleet-command__text">{text}</code>
      <button type="button" className="button fleet-command__copy" onClick={copy}><Icon name="copy" className="size-4" />{done ? t('address-copied') : t('address-copy')}</button>
    </div>
  );
}

// What the page shows when there is no fleet yet: it says where things stand and offers the next step.
export function FleetEmpty({ kind, machines, onStart, onRetry }: { kind: 'connecting' | 'offline' | 'empty' | 'first-run'; machines: readonly MachineModel[]; onStart?: (machine?: string) => void; onRetry?: () => void }) {
  const host = typeof window !== 'undefined' && window.location?.host ? window.location.host : 'HUB:7443';
  const online = machines.filter((m) => m.online);
  if (kind === 'connecting') return <div className="fleet-empty" role="status" aria-busy="true"><p className="muted m-0">{t('fleet-loading')}</p></div>;
  if (kind === 'offline') {
    return (
      <div className="fleet-empty" role="alert">
        <Icon name="machine" className="size-8 text-muted" />
        <h2 className="fleet-empty__title">{t('fleet-offline-title')}</h2>
        <p className="fleet-empty__text">{t('fleet-offline-text')}</p>
        {onRetry && <button type="button" className="button button--primary" onClick={onRetry}>{t('link-retry')}</button>}
      </div>
    );
  }
  if (kind === 'first-run') {
    return (
      <div className="fleet-empty">
        <Icon name="terminal" className="size-8 text-accent" />
        <h2 className="fleet-empty__title">{t('fleet-firstrun-title')}</h2>
        {online.length === 0 ? <p className="fleet-empty__text">{t('fleet-firstrun-offline')}</p> : <>
          <p className="fleet-empty__text">{t('fleet-firstrun-text')}</p>
          {onStart && <button type="button" className="button button--primary" onClick={() => onStart(online.length === 1 ? online[0]?.id : undefined)}><Icon name="plus" />{t('action-start-agent')}</button>}
          <div className="fleet-empty__step"><span className="muted">{t('fleet-firstrun-cli')}</span><Command text="agora agent start codex --cwd /path/to/project" /></div>
        </>}
        <a className="link" href="#/machines">{t('fleet-empty-machines')}</a>
      </div>
    );
  }
  return (
    <div className="fleet-empty">
      <Icon name="machine" className="size-8 text-accent" />
      <h2 className="fleet-empty__title">{t('fleet-empty-title')}</h2>
      <p className="fleet-empty__text">{t('fleet-empty-text')}</p>
      <div className="fleet-empty__step"><span className="muted">1 · {t('fleet-empty-step-code')}</span><Command text="agora hub code" /></div>
      <div className="fleet-empty__step"><span className="muted">2 · {t('fleet-empty-step-join')}</span><Command text={`agora node enroll --hub wss://${host} --ca ca.pem --code XXXX-XXXX --id NAME`} /><span className="muted">{t('fleet-empty-ca')}</span></div>
      <a className="link" href="#/machines">{t('fleet-empty-machines')}</a>
    </div>
  );
}

// Home: the fleet as a console (docs/ux/visual-language.md, "Glance"). Everything is derived from the props (render is pure): the counts,
// the queue of what needs you and the tiles are one memoised pass over the list; the clock only moves the times.
export function FleetConsole({ fleet, machines, approvals, usage, link, now, onOpen, onStart, onResolve, onReview, onRetry }: {
  fleet: readonly AgentTileModel[];
  machines: readonly MachineModel[];
  approvals: readonly ApprovalModel[];
  usage?: UsageSummary;
  link: LinkState;
  now: number;
  onOpen: (agentId: string) => void;
  onStart?: (machine?: string) => void;
  onResolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice) => void;
  onReview?: () => void;
  onRetry?: () => void;
}) {
  const view = useMemo(() => fleetView(fleet, approvals, machines), [fleet, approvals, machines]);
  const [filter, setFilter] = useState<FleetFilter | undefined>();
  const [by, setBy] = useState<GroupBy>(() => loadGroupBy());
  const chooseBy = (next: GroupBy) => { setBy(next); saveGroupBy(next); };
  const groups = useMemo(() => groupFleet(view.sorted, machines, by, filter), [view.sorted, machines, by, filter]);
  const empty = emptyKind(link, fleet.length + approvals.length, machines.length);
  const asks = useMemo(() => {
    const m = new Map<string, string>();
    for (const x of approvals) if (!m.has(x.agentId)) m.set(x.agentId, askText(x));
    return m;
  }, [approvals]);
  const cost = (a: AgentTileModel) => (usage?.perAgent.has(a.id) ? money(usage.perAgent.get(a.id) ?? 0) : undefined);

  const body = empty ? <FleetEmpty kind={empty} machines={machines} onStart={onStart} onRetry={onRetry} /> : (
    <>
      <AttentionQueue items={view.attention} machines={machines} now={now} onOpen={onOpen} onResolve={onResolve} onReview={onReview} stale={link !== 'live'} />
      <section className="fleet-groups" aria-label={t('fleet-panel')}>
        <div className="fleet-tools">
          <div className="fleet-tools__by" role="group" aria-label={t('fleet-group-by')}>
            <span className="muted">{t('fleet-group-by')}</span>
            {(['machine', 'project'] as const).map((g) => <button key={g} type="button" className="fleet-seg" aria-pressed={by === g} onClick={() => chooseBy(g)}>{t(g === 'machine' ? 'fleet-group-machine' : 'fleet-group-project')}</button>)}
          </div>
          {filter && <p className="fleet-tools__filter m-0" role="status">{t('fleet-filter-active', { name: t(FILTER_WORD[filter]) })} <button type="button" className="link" onClick={() => setFilter(undefined)}>{t('fleet-filter-clear')}</button></p>}
        </div>
        {groups.length === 0 && <p className="muted m-0">{t('fleet-filter-none')}</p>}
        {groups.map((g) => (
          <section key={g.key} className={`fleet-group${g.online ? '' : ' fleet-group--offline'}`} aria-label={g.name || t('fleet-no-project')}>
            <header className="fleet-group__head">
              {by === 'machine' && <span aria-hidden className={`fleet-dot ${g.online ? 'bg-finished' : 'bg-unknown'}`} />}
              <h3 className="fleet-group__name">{g.name || t('fleet-no-project')}</h3>
              <span className="muted">{t('ide-status-agents', { count: g.agents.length })}</span>
              {by === 'machine' && !g.online && g.machine && <span className="muted">{t('fleet-offline-since', { when: ago(g.machine.lastSeenMs, now) })}</span>}
              <span className="flex-1" />
              {by === 'machine' && g.online && onStart && <button type="button" className="ide-iconbtn" onClick={() => onStart(g.machine?.id)} aria-label={t('fleet-start-on', { machine: g.name })} title={t('fleet-start-on', { machine: g.name })}><Icon name="plus" /></button>}
            </header>
            {by === 'machine' && !g.online && <p className="muted m-0">{t('fleet-offline-note')}</p>}
            {g.agents.length === 0 ? (g.online ? <p className="muted m-0">{t('fleet-no-agents-here')}</p> : null) : (
              <div className="fleet-grid">
                {g.agents.map((a) => <AgentTile key={a.id} agent={a} onOpen={onOpen} context={{ group: by, ask: asks.get(a.id), cost: cost(a), now }} />)}
              </div>
            )}
          </section>
        ))}
      </section>
    </>
  );

  return (
    <div className="fleet-console">
      {!empty && <div className="fleet-console__bar"><FleetSummary counts={view.counts} filter={filter} onFilter={setFilter} /></div>}
      {link === 'offline' && !empty && (
        <p role="status" className="fleet-stale">
          <span>{t('fleet-offline-stale')}</span>
          {onRetry && <button type="button" className="button" onClick={onRetry}>{t('link-retry')}</button>}
        </p>
      )}
      {body}
    </div>
  );
}
