import { useEffect, useId, useMemo, useState, type ReactNode } from 'react';
import { shownKey, useKeymap } from '../keys/view';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { type AttentionItem, type FleetFilter, FLEET_FILTERS, matchesModelPick, modelPicks, projectSpread, sharedAcross, type ProjectSpread, type FleetCounts, collapseAttention, decidableInline, emptyKind, fleetView, groupFleet } from '../net/fleet';
import { saveGroupBy, useGroupBy } from '../net/fleetPrefs';
import { GroupByControl, groupTitle } from './GroupByControl';
import { ProjectChip, ProjectHint } from './ProjectMark';
import { ago } from '../net/time';
import type { UsageSummary } from '../net/usage';
import { approvalName } from '../net/approvalText';
import { detailText } from '../net/tool';
import { type ApprovalModel, type MachineModel, visible } from '../net/view';
import { AgentTile, kindOf } from './AgentTile';
import { ModelPick } from './ModelPick';
import { Skeleton } from './EmptyState';
import { Icon, type IconName } from './Icon';
import { AgentAvatar, Button, CopyChip, Pill, StatTile, stateTone } from './kit';
import { AnswerButtons, ApprovalRequest, ASK_ICON, askKind, askTitle } from './ApprovalRequest';
import { Mark } from './StateBadge';
import { ReviewDialog } from './ReviewRequest';
import { ShareHolds, useAnswerGuard } from './useAnswerGuard';
import type { StartChoice } from './StartApproval';
import { PairRecentList, PairWindowControl, type PairingUi } from './Pairing';
import { PairRequest } from './PairRequest';
import { isolated, peerText } from './pairChoice';

export const ATTENTION_SHOWN = 6;

export type LinkState = 'connecting' | 'live' | 'offline';

// What one filter chip says, in the state words the rest of the window uses.
const FILTER_WORD: Record<FleetFilter, string> = { 'needs-you': 'state-needs-you', working: 'state-working', finished: 'state-finished', failed: 'fleet-failed' };
const countOf = (c: FleetCounts, f: FleetFilter): number => (f === 'needs-you' ? c.needs : c[f]);

// The header: five answers, each a live count, as stat tiles (only Needs you is tinted, and only above zero). Four are filters of the tiles
// below (a press shows only those, a second press shows everything again); the machines count opens the machines page. A count of zero is
// quiet and not pressable, unless it is the filter in use (then it can still be turned off).
export function FleetSummary({ counts, filter, onFilter }: { counts: FleetCounts; filter?: FleetFilter; onFilter?: (f: FleetFilter | undefined) => void }) {
  const machines = counts.machinesTotal === 0 ? t('machines-online', { count: 0 }) : t('fleet-machines', { online: counts.machinesOnline, total: counts.machinesTotal });
  return (
    <div className="fleet-summary" role="group" aria-label={t('fleet-summary-label')}>
      {FLEET_FILTERS.map((f) => {
        const n = countOf(counts, f);
        const on = filter === f;
        const press = onFilter && (n > 0 || on) ? () => onFilter(on ? undefined : f) : undefined;
        // The mark's shape beside the word; the word is the name (the mark adds nothing for a screen reader).
        const mark = <span className={`kit-mark kit-tone-${f === 'failed' ? 'danger' : stateTone(f)}`}><Mark state={f === 'failed' ? 'finished' : f} failed={f === 'failed'} /></span>;
        return <StatTile key={f} value={n} label={t(FILTER_WORD[f])} mark={mark} tone={f === 'needs-you' ? 'needs' : undefined} pressed={on} onPress={press}
          title={press ? `${t('fleet-filter-hint')}: ${t(FILTER_WORD[f])}` : undefined} />;
      })}
      <a className={`kit-stat fleet-stat--machines${counts.machinesOnline === 0 ? ' kit-stat--zero' : ''}`} href="#/machines" aria-label={machines} title={machines}>
        <span className="kit-stat__n">{counts.machinesOnline}</span>
        <span className="kit-stat__label"><Icon name="machine" className="size-3.5" />{t('fleet-stat-machines', { total: counts.machinesTotal })}</span>
      </a>
    </div>
  );
}

// The one line of what an agent asks: the request's own words, short. Anything long goes through the full card. A start request's title
// already says it wants to start another agent, so its line is the start of its first prompt (cut: it is only ever reviewed, never answered
// here).
function askText(x: ApprovalModel): string {
  // A switch to act without asking: what it would mean, in the window's words (the title names it; the ids are read in its full card).
  if (x.skip) return t('skip-approval-body');
  if (x.start) return x.start.prompt.trim() ? visible(x.start.prompt.replace(/\s+/g, ' ').trim()) : t('start-approval-no-prompt');
  const d = x.detail ? detailText(x.detail) : '';
  return visible(d ? `${x.action} · ${d}` : x.action);
}

// A start or a switch to act without asking is read and answered in Review (#1070); a switch the window cannot read keeps its full card.
const reviewsHere = (x: ApprovalModel): boolean => !!(x.start || x.skip);

const askOf = (it: AttentionItem): string => (it.kind === 'approval' ? askText(it.approval) : it.kind === 'pair' ? t('pair-row-what') : t('fleet-waiting-in-terminal'));

// The icon chip of a request card: what kind of thing is asked.
function iconOf(it: AttentionItem): IconName {
  if (it.kind === 'pair') return 'machine';
  if (it.kind === 'agent') return 'terminal';
  return ASK_ICON[askKind(it.approval)];
}

// What one agent's card shows, worked out once: the card is drawn from it and the dwell's identity is made of it, so nothing on the card
// can change without the dwell starting again (Codex review of #1028).
interface RowModel {
  name: string;
  title: string; // the ask itself ("codex · teal wants to run a command"), or the agent's name when it waits in its terminal
  icon: IconName;
  machine?: string;
  project?: string;
  ask: string;
  inline: boolean; // Allow and Deny are on the card
  when?: string; // "Asked 3 minutes ago", when the queue shows it (drawn in a box of its own width: not part of the dwell's identity)
  action: 'answer' | 'review' | 'agent' | 'panel' | 'none';
  agentId?: string;
}

// The queue: everything that needs the user, most urgent first, one request card each (the kit's Callout, amber, never a bar on one side).
// A plain yes or no has Allow and Deny in its card (the same answer, signed the same way, as the banner above the terminal); anything with
// something to read opens the agent, where the whole request is. A start or a switch (#1070) opens Review here, over Home: the same dialog
// the agent's card opens, so it takes one Review wherever it is seen.
export function AttentionQueue({ items, machines, now, onOpen, onResolve, onReview, stale, pairing }: {
  items: readonly AttentionItem[];
  machines: readonly MachineModel[];
  now: number;
  onOpen: (agentId: string) => void;
  // What it returns says how the answer went: `false` (or a promise of it, or one that rejects) is a failed answer.
  onResolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => void | boolean | PromiseLike<void | boolean>;
  // Where a request is read in full when this window does not list its agent (so there is no agent to open).
  onReview?: () => void;
  // The link is not live: an answer cannot be signed to a machine through it, so Allow and Deny wait, with the reason shown.
  stale?: boolean;
  // ADR-0065: answers a machine's request to join, in the full card the row opens.
  pairing?: PairingUi;
}) {
  const machineName = (id: string) => machines.find((m) => m.id === id)?.name;
  // The key of "Next that needs me" as it is now (N unless the person changed it; none, no hint).
  const nextKey = shownKey(useKeymap(), 'attention.next');
  // A glance is a few rows: the most urgent ones, and the rest one press away. Many identical asks are one row.
  const [all, setAll] = useState(false);
  const [each, setEach] = useState<ReadonlySet<string>>(new Set());
  const rows = useMemo(() => collapseAttention(items), [items]);
  const shown = all ? rows : rows.slice(0, ATTENTION_SHOWN);
  // How long it has waited is shown on every row or on none: a list where some rows say it and others do not reads as if those were fresh.
  const knownWait = items.length > 0 && items.every((x) => x.kind !== 'pair' && !!x.sinceMs);
  const toggleEach = (key: string) => setEach((cur) => { const next = new Set(cur); if (!next.delete(key)) next.add(key); return next; });

  // A request to join is a row with one button, Review: its full card (name, system, address, number, Approve) opens under the row. It
  // never has Approve in the row, because the owner must see the number first.
  const [read, setRead] = useState<ReadonlySet<string>>(new Set());
  const toggleRead = (key: string) => setRead((cur) => { const next = new Set(cur); if (!next.delete(key)) next.add(key); return next; });

  // #1070: the start or switch whose Review is open over Home, by request id. It is drawn from the list as it is now, so a request that
  // leaves the list takes the dialog with it.
  const [reviewing, setReviewing] = useState<string | undefined>();
  const reviewed = reviewing === undefined ? undefined
    : items.find((it): it is Extract<AttentionItem, { kind: 'approval' }> => it.kind === 'approval' && it.approval.id === reviewing && reviewsHere(it.approval));
  // Gone from the list: forgotten, so the same request asked again later does not open Review by itself.
  useEffect(() => { if (reviewing !== undefined && !reviewed) setReviewing(undefined); }, [reviewing, reviewed]);

  // T44 (`useAnswerGuard`, shared with the side panel and the cards above a terminal and a composer): Allow and Deny in a card do nothing
  // for a short dwell after anything on the list changed, after the person came back to the window and after the link came back, and a
  // request is answered once. The layout is made of the model of every agent card on screen, opened "Review each" lists included (the
  // cards are drawn from these same models), and of each card's status, so an answer settling restarts the dwell.
  const model = (it: Exclude<AttentionItem, { kind: 'pair' }>, inlineOk: boolean): RowModel => {
    const agent = it.agent;
    const name = visible(it.kind === 'approval' ? (agent?.name || approvalName(it.approval)) : it.agent.name);
    const inline = inlineOk && it.kind === 'approval' && !!onResolve && decidableInline(it.approval);
    const review = !inline && it.kind === 'approval' && reviewsHere(it.approval);
    return {
      name,
      title: it.kind === 'approval' ? askTitle(it.approval, name) : name,
      icon: iconOf(it),
      machine: visible(agent?.machine || (it.kind === 'approval' ? it.approval.machineName || machineName(it.approval.node) || '' : '')) || undefined,
      project: agent ? visible(agent.project || agent.workspace) : undefined,
      ask: askOf(it),
      inline,
      when: knownWait && it.sinceMs ? t('fleet-asked', { when: ago(it.sinceMs, now) }) : undefined,
      action: inline ? 'answer' : review ? 'review' : agent ? 'agent' : onReview ? 'panel' : 'none',
      agentId: agent?.id,
    };
  };
  const guard = useAnswerGuard(items.flatMap((it) => (it.kind === 'approval' ? [it.approval.id] : [])), (statusOf) => {
    const identity = (it: AttentionItem, inlineOk: boolean): string => (it.kind === 'pair'
      ? JSON.stringify([it.key, peerText(it.card.name), read.has(it.key)])
      : JSON.stringify([it.key, it.kind === 'approval' ? [it.approval.action, it.approval.detail, statusOf(it.approval.id)] : '', { ...model(it, inlineOk), when: !!model(it, inlineOk).when }]));
    // Review opening or closing over Home starts the dwell again: a second click on its Close never lands on an Allow in a row under it.
    return JSON.stringify([all, stale, reviewing ?? null, shown.map((r) => (r.kind === 'one' ? identity(r.item, true)
      : [r.key, r.items.length, r.agentKind, r.machine, askOf(r.items[0] as AttentionItem), each.has(r.key) ? r.items.map((it) => identity(it, false)) : null]))]);
  }, stale);
  const { canAnswer } = guard;
  const sentTo = (id: string) => guard.status(id) === 'sent';
  const answer = (a: ApprovalModel, ok: boolean) => guard.answer(a.id, () => onResolve?.(a, ok));

  // With a side panel (Review opens it, where the full card is) the card is brought into view there; without one it opens under its row.
  const reviewPair = (key: string, id: string) => {
    if (onReview) {
      onReview();
      if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => { const el = document.getElementById(`pair-request-${id}`); el?.scrollIntoView?.({ block: 'nearest' }); el?.focus(); });
      return;
    }
    toggleRead(key);
  };
  // Every card of the queue is the one request component in its row density (redesign PR 4).
  const card = (key: string, cls: string, icon: IconName, title: ReactNode, body: ReactNode, actions: ReactNode, avatar?: ReactNode, where?: ReactNode) => (
    <li key={key} className={cls}><ApprovalRequest density="row" icon={icon} avatar={avatar} title={title} where={where} actions={actions}>{body}</ApprovalRequest></li>
  );
  const pairRow = (it: Extract<AttentionItem, { kind: 'pair' }>) => {
    const open = read.has(it.key);
    const c = it.card;
    return card(it.key, `fleet-ask${open ? ' fleet-ask--open' : ''}`, 'machine',
      <bdi className="fleet-ask__name">{peerText(c.name)}</bdi>,
      <>
        <p className="fleet-ask__what">{t('pair-row-what')}</p>
        {open && pairing && <div id={`pair-row-${c.id}`} className="mt-2"><PairRequest inline request={c} offline={pairing.offline} onAnswer={(a, r) => pairing.onAnswer(c, a, r)} /></div>}
      </>,
      <Button size="sm" variant="primary" aria-expanded={onReview ? undefined : open} aria-controls={onReview ? `pair-request-${c.id}` : `pair-row-${c.id}`} aria-label={t('pair-review-for', { name: isolated(c.name) })} onClick={() => reviewPair(it.key, c.id)}>{open && !onReview ? t('pair-hide') : t('fleet-review')}</Button>,
      undefined, <span className="fleet-ask__where">{t('pair-row-where')}</span>);
  };
  const rowOf = (it: AttentionItem, inlineOk: boolean) => {
    if (it.kind === 'pair') return pairRow(it);
    const agent = it.agent;
    const { name, title, icon, machine, project, ask, inline, when, action, agentId } = model(it, inlineOk);
    const why = `fleet-stale-why-${it.key}`;
    const off = { 'aria-disabled': !(it.kind === 'approval' && canAnswer(it.approval.id)) || undefined, disabled: stale, 'aria-describedby': stale ? why : undefined };
    return card(it.key, 'fleet-ask', icon,
      <bdi className="fleet-ask__name">{title}</bdi>,
      <>
        {/* A request that can be answered here is shown whole, never cut; the others may be cut because they are read in the full card. */}
        <p className={`fleet-ask__what${inline ? '' : ' fleet-ask__what--clamp'}`} title={ask}>{inline ? <span className="fleet-ask__cmd" dir="auto">{ask}</span> : ask}</p>
        {inline && stale && <span id={why} className="fleet-ask__why">{t('fleet-stale-no-answer')}</span>}
        {it.kind === 'approval' && sentTo(it.approval.id) && <span role="status" className="fleet-ask__why">{t('attention-sent')}</span>}
      </>,
      // "Always" is never answered from a row (ADR-0101: `decidableInline` sends a request that offers only that to its full card).
      action === 'answer' && it.kind === 'approval' ? <>
        <AnswerButtons approval={it.approval} size="sm" off={off} denyLabel={t('fleet-deny-for', { name })} allowLabel={t('fleet-allow-for', { name })}
          onDeny={() => answer(it.approval, false)} onAllow={() => answer(it.approval, true)} />
        {sentTo(it.approval.id) && agentId && <Button size="sm" aria-label={t('fleet-review-for', { name })} onClick={() => onOpen(agentId)}>{t('attention-review')}</Button>}
      </> : action === 'review' && it.kind === 'approval' ? <Button size="sm" variant="primary" aria-haspopup="dialog" aria-label={t('fleet-review-for', { name })} onClick={() => setReviewing(it.approval.id)}>{t('fleet-review')}</Button>
        : action === 'agent' && agentId ? <Button size="sm" variant="primary" aria-label={t(it.kind === 'approval' ? 'fleet-review-for' : 'fleet-open-for', { name })} onClick={() => onOpen(agentId)}>{t(it.kind === 'approval' ? 'fleet-review' : 'action-open')}</Button>
        : action === 'panel' && onReview ? <Button size="sm" variant="primary" aria-label={t('fleet-review-for', { name })} onClick={onReview}>{t('fleet-review')}</Button> : null,
      <AgentAvatar accent={agent?.accent ?? 'var(--agent-slate)'} kind={kindOf({ kind: agent?.kind, name })} size="s" />,
      <>
        <span className="fleet-ask__where">{[machine, project].filter(Boolean).join(' · ')}</span>
        {when && it.sinceMs ? <time className="fleet-ask__time" dateTime={new Date(it.sinceMs).toISOString()}>{when}</time> : null}
      </>);
  };

  return (
    <section className="fleet-attention" aria-labelledby="fleet-attention-h">
      <div className="fleet-section__head">
        <h2 id="fleet-attention-h" className="fleet-section__title">{t('attention-title')}{items.length > 0 && <Pill tone="needs"><span className="sr-only">· </span>{items.length}</Pill>}</h2>
        {items.length > 0 && nextKey && <span className="fleet-hint"><kbd className="ide-kbd">{nextKey}</kbd> {t('ide-welcome-next')}</span>}
      </div>
      {items.length === 0 ? <p className="fleet-calm muted m-0">{t('attention-empty')}</p> : (
        <ul className="fleet-queue">
          {shown.map((row) => row.kind === 'one' ? rowOf(row.item, true) : card(row.key, `fleet-ask fleet-ask--many${each.has(row.key) ? ' fleet-ask--open' : ''}`, 'stack',
            <span className="fleet-ask__name">{row.agentKind ? t('fleet-same-ask', { count: row.items.length, kind: visible(row.agentKind), machine: visible(row.machine) }) : t('fleet-same-ask-any', { count: row.items.length, machine: visible(row.machine) })}</span>,
            <>
              <p className="fleet-ask__what fleet-ask__what--clamp" title={askOf(row.items[0] as AttentionItem)}>{askOf(row.items[0] as AttentionItem)}</p>
              {each.has(row.key) && <ul className="fleet-queue fleet-ask__each">{row.items.map((it) => rowOf(it, false))}</ul>}
            </>,
            <Button size="sm" aria-expanded={each.has(row.key)} onClick={() => toggleEach(row.key)}>{each.has(row.key) ? t('fleet-collapse') : t('fleet-review-each')}</Button>))}
        </ul>
      )}
      {rows.length > ATTENTION_SHOWN && <Button className="fleet-more" aria-expanded={all} onClick={() => setAll((v) => !v)}>{all ? t('fleet-attention-less') : t('fleet-attention-more', { count: rows.length - ATTENTION_SHOWN })}</Button>}
      {/* Its own dwell and guards (a start's `useAnswerGuard`, a switch's `useSkipAnswer`), in this list's holds (the window's), with the
          raw answer: the dialog's guard holds the request, so this list's guard must not hold it first. */}
      {reviewed && <ShareHolds holds={guard.holds}>
        <ReviewDialog approval={reviewed.approval} agent={reviewed.agent} stale={!!stale} onResolve={onResolve ? (ok, choice) => onResolve(reviewed.approval, ok, choice) : undefined} onClose={() => setReviewing(undefined)} />
      </ShareHolds>}
    </section>
  );
}

// How another machine joins: a join code here, the command there, and (for the owner) the window that lets a machine join without a code.
function JoinSteps({ pairing }: { pairing?: PairingUi }) {
  const host = typeof window !== 'undefined' && window.location?.host ? window.location.host : 'HUB:7443';
  return (
    <>
      <div className="fleet-empty__step"><span className="fleet-empty__step-label">1 · {t('fleet-empty-step-code')}</span><CopyChip text="agora hub code" /></div>
      <div className="fleet-empty__step"><span className="fleet-empty__step-label">2 · {t('fleet-empty-step-join')}</span><CopyChip text={`agora node enroll --hub wss://${host} --ca ca.pem --code XXXX-XXXX --id NAME`} /><span className="fleet-empty__step-label">{t('fleet-empty-ca')}</span></div>
      {/* ADR-0065: the owner at this window opens the 10-minute window for a new machine (its own button); `agora join` there does the rest. */}
      {pairing && <div className="fleet-empty__step"><PairWindowControl ui={pairing} intro /></div>}
    </>
  );
}

// What the page shows when there is no fleet yet, centred: where things stand and the next step. Commands are shown as they are typed.
// `guest`: the hub said this session is not the owner's (ADR-0049). A person sees only the projects the owner gave them, so an empty
// window means no project is theirs yet, and the owner's setup (join codes, enrollment) is not theirs to do (#904).
// `watching`: the hub said this person is a viewer (ADR-0106): nothing to start, so the first run says what they will see instead.
export function FleetEmpty({ kind, machines, onStart, onRetry, pairing, guest, watching }: { kind: 'connecting' | 'offline' | 'empty' | 'first-run'; machines: readonly MachineModel[]; onStart?: (machine?: string) => void; onRetry?: () => void; pairing?: PairingUi; guest?: boolean; watching?: boolean }) {
  const [adding, setAdding] = useState(false);
  const joinId = useId();
  const online = machines.filter((m) => m.online);
  const joining = adding || !!pairing?.window?.open;
  if (kind === 'connecting') return <div className="fleet-empty fleet-empty--wide" role="status" aria-busy="true"><p className="muted m-0">{t('fleet-loading')}</p>
      <div className="fleet-grid w-full" aria-hidden>{[0, 1, 2].map((i) => <div key={i} className="tile tile--compact"><Skeleton className="h-4 w-2/3" /><Skeleton className="h-3 w-full" /><Skeleton className="h-3 w-1/2" /></div>)}</div></div>;
  if (kind === 'offline') {
    return (
      <div className="fleet-empty kit-tone-idle" role="alert">
        <span aria-hidden className="kit-chip"><Icon name="machine" className="size-5" /></span>
        <h2 className="fleet-empty__title">{t('fleet-offline-title')}</h2>
        <p className="fleet-empty__text">{t('fleet-offline-text')}</p>
        {onRetry && <div className="fleet-empty__actions"><Button variant="primary" onClick={onRetry}>{t('link-retry')}</Button></div>}
      </div>
    );
  }
  if (kind === 'first-run' && watching) {
    return (
      <div className="fleet-empty kit-tone-accent">
        <span aria-hidden className="kit-chip"><Icon name="terminal" className="size-5" /></span>
        <h2 className="fleet-empty__title">{t('fleet-watch-title')}</h2>
        <p className="fleet-empty__text">{t('fleet-watch-text')}</p>
      </div>
    );
  }
  if (kind === 'first-run') {
    return (
      <div className="fleet-empty kit-tone-accent">
        <span aria-hidden className="kit-chip"><Icon name="terminal" className="size-5" /></span>
        <h2 className="fleet-empty__title">{t('fleet-firstrun-title')}</h2>
        <p className="fleet-empty__text">{t(online.length === 0 ? 'fleet-firstrun-offline' : 'fleet-firstrun-text')}</p>
        <div className="fleet-empty__actions">
          {online.length > 0 && onStart && <Button variant="primary" onClick={() => onStart(online.length === 1 ? online[0]?.id : undefined)}><Icon name="plus" />{t('action-start-agent')}</Button>}
          {/* Shows how a machine joins, here; it opens nothing by itself (the window for new machines has its own button). Not for a
              guest: adding a machine is the owner's setup (#904). */}
          {!guest && <Button aria-expanded={joining} aria-controls={joinId} onClick={() => setAdding(!adding)}><Icon name="machine" />{t('fleet-add-machine')}</Button>}
        </div>
        {online.length > 0 && <div className="fleet-empty__step"><span className="fleet-empty__step-label">{t('fleet-firstrun-cli')}</span><CopyChip text="agora agent start codex --cwd /path/to/project" /></div>}
        {!guest && <div id={joinId} hidden={!joining} className="fleet-empty__join">{joining && <JoinSteps pairing={pairing} />}</div>}
      </div>
    );
  }
  if (guest) {
    return (
      <div className="fleet-empty">
        <Icon name="machine" className="size-8 text-muted" />
        <h2 className="fleet-empty__title">{t('fleet-guest-title')}</h2>
        <p className="fleet-empty__text">{t('fleet-guest-text')}</p>
      </div>
    );
  }
  return (
    <div className="fleet-empty kit-tone-accent">
      <span aria-hidden className="kit-chip"><Icon name="machine" className="size-5" /></span>
      <h2 className="fleet-empty__title">{t('fleet-empty-title')}</h2>
      <p className="fleet-empty__text">{t('fleet-empty-text')}</p>
      <JoinSteps pairing={pairing} />
      <a className="link mt-2" href="#/machines">{t('fleet-empty-machines')}</a>
    </div>
  );
}

// Home: the fleet as a console (docs/ux/visual-language.md, "Glance"). Everything is derived from the props (render is pure): the counts,
// the queue of what needs you and the tiles are one memoised pass over the list; the clock only moves the times.
export function FleetConsole({ fleet, machines, approvals, usage, link, now, onOpen, onStart, onResolve, onReview, onRetry, pairing, guest, watching }: {
  fleet: readonly AgentTileModel[];
  machines: readonly MachineModel[];
  approvals: readonly ApprovalModel[];
  usage?: UsageSummary;
  link: LinkState;
  now: number;
  onOpen: (agentId: string) => void;
  onStart?: (machine?: string) => void;
  onResolve?: (a: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => void;
  onReview?: () => void;
  onRetry?: () => void;
  // ADR-0065: machines asking to join (only for an owner's session), and the switch that lets them.
  pairing?: PairingUi;
  // The hub said this session is not the owner's (see FleetEmpty).
  guest?: boolean;
  watching?: boolean;
}) {
  const view = useMemo(() => fleetView(fleet, approvals, machines, pairing?.cards), [fleet, approvals, machines, pairing?.cards]);
  const [filter, setFilter] = useState<FleetFilter | undefined>();
  const by = useGroupBy();
  const spread = useMemo(() => projectSpread(fleet), [fleet]);
  // ADR-0123: one provider or one model at a time; offered once the agents run on more than one.
  const [pick, setPick] = useState<string | undefined>();
  const picks = useMemo(() => modelPicks(view.sorted), [view.sorted]);
  const picked = useMemo(() => (pick ? view.sorted.filter((a) => matchesModelPick(a, pick)) : view.sorted), [view.sorted, pick]);
  const groups = useMemo(() => groupFleet(picked, machines, by, filter), [picked, machines, by, filter]);
  const empty = emptyKind(link, fleet.length + approvals.length, machines.length);
  const asks = useMemo(() => {
    const m = new Map<string, string>();
    for (const x of approvals) if (!m.has(x.agentId)) m.set(x.agentId, askText(x));
    return m;
  }, [approvals]);

  const body = empty ? (
    <>
      {view.attention.length > 0 && <AttentionQueue items={view.attention} machines={machines} now={now} onOpen={onOpen} onResolve={onResolve} onReview={onReview} stale={link !== 'live'} pairing={pairing} />}
      {pairing && <PairRecentList recent={pairing.recent} />}
      <FleetEmpty kind={empty} machines={machines} onStart={onStart} onRetry={onRetry} pairing={pairing && (empty === 'empty' || empty === 'first-run') && link === 'live' ? pairing : undefined} guest={guest} watching={watching} />
    </>
  ) : (
    <>
      <AttentionQueue items={view.attention} machines={machines} now={now} onOpen={onOpen} onResolve={onResolve} onReview={onReview} stale={link !== 'live'} pairing={pairing} />
      {pairing && <PairRecentList recent={pairing.recent} />}
      {pairing?.window?.open && <PairWindowControl ui={pairing} />}
      <section className="fleet-groups" aria-label={t('fleet-panel')}>
        <div className="fleet-tools">
          <GroupByControl by={by} onChange={saveGroupBy} />
          {(pick || picks.providers.length > 1 || picks.models.length > 1) && <ModelPick value={pick} picks={picks} onChange={setPick} />}
          {filter && <p className="fleet-tools__filter m-0" role="status">{t('fleet-filter-active', { name: t(FILTER_WORD[filter]) })} <button type="button" className="link" onClick={() => setFilter(undefined)}>{t('fleet-filter-clear')}</button></p>}
        </div>
        {groups.length === 0 && <p className="muted m-0">{t('fleet-filter-none')}</p>}
        {groups.map((g) => (
          <section key={g.key} className={`fleet-group${g.online ? '' : ' fleet-group--offline'}`} aria-label={groupTitle(g, by)}>
            <header className="fleet-group__head">
              {by === 'machine' && <span aria-hidden className={`fleet-dot ${g.online ? 'bg-finished' : 'bg-unknown'}`} />}
              <h3 className="fleet-group__name">{groupTitle(g, by)}</h3>
              <span className="muted">{t('ide-status-agents', { count: g.agents.length })}</span>
              {by === 'project' && sharedAcross(spread.get(g.key)) && <ProjectChip spread={spread.get(g.key) as ProjectSpread} />}
              {by === 'machine' && !g.online && g.machine && <span className="muted">{t('fleet-offline-since', { when: ago(g.machine.lastSeenMs, now) })}</span>}
              <span className="flex-1" />
              {by === 'machine' && g.online && onStart && <button type="button" className="ide-iconbtn" onClick={() => onStart(g.machine?.id)} aria-label={t('fleet-start-on', { machine: visible(g.name) })} title={t('fleet-start-on', { machine: visible(g.name) })}><Icon name="plus" /></button>}
            </header>
            {by === 'machine' && !g.online && <p className="muted m-0">{t('fleet-offline-note')}</p>}
            {by === 'project' && sharedAcross(spread.get(g.key)) && <p className="muted m-0 text-xs">{t('fleet-same-project-on', { machines: (spread.get(g.key) as ProjectSpread).machines.map(visible).join(', ') })}</p>}
            {by === 'project' && spread.get(g.key) && <ProjectHint spread={spread.get(g.key) as ProjectSpread} />}
            {g.agents.length === 0 ? (g.online ? <p className="muted m-0">{t('fleet-no-agents-here')}</p> : null) : (
              <div className="fleet-grid">
                {g.agents.map((a) => <AgentTile key={a.id} agent={a} onOpen={onOpen} context={{ group: by, ask: asks.get(a.id), now, same: by !== 'project' && a.projectId && sharedAcross(spread.get(a.projectId)) ? spread.get(a.projectId) : undefined }} />)}
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
