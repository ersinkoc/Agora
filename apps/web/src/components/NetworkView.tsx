import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { t } from '../i18n';
import { Menu } from '../layout/Menu';
import { endedWithError, machineKey, type AgentTileModel } from '../model';
import { bytesLabel, isOld, type MachineModel } from '../net/view';
import { ForgetMachines } from './ForgetMachines';
import { Icon } from './Icon';
import { AgentAvatar, Button, Callout, ListRow, Pill, StatusPill } from './kit';
import { NoAskBadge } from './NoAskBadge';
import { PairWindowControl, type PairingUi } from './Pairing';
import { PairRequest } from './PairRequest';
import { MachineFacts, MachineName, OsTile } from './machineFacts';
import { isolated } from './pairChoice';

// My machines (redesign PR 9): every machine your Agora knows, as a card. What is connected now, what runs there, when it was last
// heard, and the two things to do on it. A machine that asks to join is a request card at the top, with the same two presses as
// everywhere. Nothing here moves on its own: the old diagram's animated links are gone (visual-language.md, "nothing decorative loops").
// ADR-0073: machines not seen for a day are listed apart, as old, folded; an offline machine can be forgotten, and the old ones cleaned
// up in one go (`onForget` absent: no such actions, as in the demo). `now` fixes the clock (the gallery); by default it ticks every 5 s.
export function NetworkView({ machines: all, agents, hubName, onForget, focus, now: clock, pairing, onStart, onNewTerminal }: {
  focus?: string;
  machines: readonly MachineModel[];
  agents: readonly AgentTileModel[];
  hubName: string;
  onForget?: (ids: string[], refuseReturn: boolean) => Promise<Map<string, string | undefined>>;
  now?: number;
  // An owner's session: requests to join and the switch for new machines. Absent: the steps to add a machine are a hint only.
  pairing?: PairingUi;
  onStart?: (node: string) => void;
  onNewTerminal?: (node: string) => void;
}) {
  const [ticked, setTicked] = useState(() => Date.now());
  const now = clock ?? ticked;
  // Arriving from the command palette: the machine asked for is brought into view and focused.
  useEffect(() => { if (focus) { const el = document.getElementById(`machine-${focus}`); el?.scrollIntoView?.({ block: 'center' }); el?.focus({ preventScroll: true }); } }, [focus, all.length]);
  useEffect(() => { if (clock !== undefined) return; const id = setInterval(() => setTicked(Date.now()), 5000); return () => clearInterval(id); }, [clock]);
  const [adding, setAdding] = useState(false);
  const addPanel = useRef<HTMLElement>(null);
  const [forgetting, setForgetting] = useState<{ list: MachineModel[]; cleanup: boolean } | undefined>();
  const machines = all.filter((m) => !isOld(m, now));
  const old = all.filter((m) => isOld(m, now));
  const live = agents.filter((a) => !a.closed);
  // Running means on a machine that is online now; what runs on an offline one is told on its own card ("N agents were running").
  const online = new Set(machines.filter((m) => m.online).map((m) => m.id));
  const running = live.filter((a) => online.has(machineKey(a))).length;
  const totalMem = machines.reduce((n, m) => n + (m.memoryBytes ?? 0), 0);
  const needs = live.filter((a) => a.state === 'needs-you').length;
  // Showing the steps never opens the switch for new machines: only its own button does (main-window.md).
  const add = () => { setAdding(true); requestAnimationFrame(() => { addPanel.current?.scrollIntoView?.({ block: 'nearest' }); addPanel.current?.focus({ preventScroll: true }); }); };

  return <div className="machines">
    <div className="machines__head">
      <p className="muted m-0 min-w-0 flex-1">{t('machines-intro')}</p>
      <Button variant="primary" aria-expanded={adding} aria-controls="machines-add" onClick={() => (adding ? setAdding(false) : add())}><Icon name="plus" />{t('machines-add')}</Button>
    </div>

    {adding && <section id="machines-add" ref={addPanel} tabIndex={-1} aria-labelledby="machines-add-title" className="kit-card machines-add">
      <h2 id="machines-add-title" className="machines-add__title">{t('machines-add')}</h2>
      <ol className="machines-add__steps">
        <li><span>{t('machines-add-step-allow')}</span>{pairing && <PairWindowControl ui={pairing} />}</li>
        <li><span>{t('machines-add-step-run')}</span><code dir="ltr" className="machines-cmd">{t('machines-add-command')}</code></li>
        <li><span>{t('machines-add-step-compare')}</span></li>
      </ol>
    </section>}

    {/* A request to join, in full: who asks (name, system and address on rows of their own), the number, then its two presses. */}
    {pairing?.cards.map((c) => <PairRequest key={c.id} inline wide now={clock} request={c} offline={pairing.offline} onAnswer={(a, r) => pairing.onAnswer(c, a, r)} />)}

    <section aria-label={t('network-hub')} className="kit-card hub-strip">
      <span aria-hidden className="hub-strip__mark">A</span>
      <div className="grid min-w-0"><h2 className="hub-strip__name">{t('network-hub')}</h2><code dir="ltr" className="truncate text-xs text-muted">{hubName}</code></div>
      <dl className="hub-strip__stats">
        <Stat value={<>{online.size} <small>{t('machines-stat-of', { total: machines.length })}</small></>} label={t('machines-stat-online', { total: machines.length })} />
        <Stat value={running} label={t('machines-stat-agents', { count: running })} />
        <Stat value={needs} label={t('machines-stat-needs', { count: needs })} needs={needs > 0} />
        {totalMem > 0 && <Stat value={bytesLabel(totalMem)} label={t('machines-stat-memory')} />}
      </dl>
    </section>

    <ul className="machines__grid">
      {machines.map((m) => <li key={m.id}>
        <MachineCard m={m} now={now} focused={focus === m.id} agents={agents.filter((a) => machineKey(a) === m.id)} share={totalMem > 0 && m.memoryBytes ? m.memoryBytes / totalMem : undefined}
          onForget={onForget ? () => setForgetting({ list: [m], cleanup: false }) : undefined} onStart={onStart} onNewTerminal={onNewTerminal} />
      </li>)}
      <li>
        <button type="button" className="kit-card machine-card machine-card--add" aria-controls="machines-add" onClick={add}>
          <span aria-hidden className="kit-chip kit-tone-accent"><Icon name="plus" className="size-5" /></span>
          <span className="machine-card__name">{t('machines-add')}</span>
          <span className="text-xs text-muted">{t('machines-add-hint')}</span>
        </button>
      </li>
    </ul>

    {old.length > 0 && <details className="kit-card machines-old">
      <summary className="cursor-pointer text-sm font-semibold">{t('old-machines-title', { count: old.length })}</summary>
      <p className="muted m-0 text-xs">{t('old-machines-explain')}</p>
      <ul className="m-0 grid list-none gap-1 p-0">
        {old.map((m) => <li key={m.id} className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <OsTile os={m.os} />
          <span className="grid min-w-0 flex-1"><MachineName m={m} /><span className="text-xs text-muted"><MachineFacts m={m} now={now} /></span></span>
          {onForget && <Button size="sm" variant="danger-ghost" onClick={() => setForgetting({ list: [m], cleanup: false })}><Icon name="trash" className="size-3.5" />{t('forget-machine-action')}</Button>}
        </li>)}
      </ul>
      {onForget && <div className="flex justify-end"><Button onClick={() => setForgetting({ list: old, cleanup: true })}>{t('old-machines-cleanup')}</Button></div>}
    </details>}
    {forgetting && onForget && <ForgetMachines machines={forgetting.list} now={now} cleanup={forgetting.cleanup} onForget={onForget} onClose={() => setForgetting(undefined)} />}
  </div>;
}

// One count in the strip: the number above its name (a `dl`, so the name is said first). Only "need you" is amber, and only above zero.
function Stat({ value, label, needs }: { value: ReactNode; label: string; needs?: boolean }) {
  return <div className="hub-stat"><dt className={needs ? 'text-needs-you' : undefined}>{label}</dt><dd>{value}</dd></div>;
}

function MachineCard({ m, now, focused, agents, share, onForget, onStart, onNewTerminal }: {
  m: MachineModel; now: number; focused: boolean; agents: readonly AgentTileModel[]; share?: number;
  onForget?: () => void; onStart?: (node: string) => void; onNewTerminal?: (node: string) => void;
}) {
  const nameId = `machine-${m.id}-name`;
  // The meter is this machine's share of the memory Agora uses on all the machines listed: the one total the window knows.
  const percent = share === undefined ? 0 : Math.max(1, Math.round(share * 100));
  return <article id={`machine-${m.id}`} tabIndex={-1} aria-labelledby={nameId} aria-current={focused ? 'true' : undefined}
    className={`kit-card machine-card${m.online ? '' : ' machine-card--away'}${focused ? ' ring-2 ring-accent' : ''}`}>
    <div className="machine-card__head">
      <OsTile os={m.os} />
      <div className="grid min-w-0 flex-1">
        <MachineName m={m} id={nameId} className="machine-card__name" />
        <span className="text-xs text-muted"><MachineFacts m={m} now={now} /></span>
      </div>
      <Pill tone={m.online ? 'finished' : 'plain'} icon={<span aria-hidden className={`size-1.5 rounded-full ${m.online ? 'bg-finished' : 'bg-unknown'}`} />}>{t(m.online ? 'machine-online' : 'machine-offline')}</Pill>
    </div>

    {m.online ? <>
      {agents.length > 0 && <ul className="m-0 grid list-none grid-cols-1 gap-0.5 p-0">
        {agents.map((a) => <li key={a.id}><ListRow href={`#/agent/${encodeURIComponent(a.id)}`} label={a.name}
          leading={<AgentAvatar accent={a.accent} kind={a.kind ?? a.name} size="s" />}
          meta={<>{!a.closed && <NoAskBadge agent={a} nested />}<StatusPill state={a.state} failed={endedWithError(a)} /></>} /></li>)}
      </ul>}
      {!!m.memoryBytes && <div className="machine-meter">
        <div className="machine-meter__row"><span>{t('machine-memory')}</span><span>{bytesLabel(m.memoryBytes)}{m.terminalsOpen ? ` · ${t('network-terminals', { count: m.terminalsOpen })}` : ''}</span></div>
        <div className="machine-meter__bar" role="img" aria-label={t('machine-memory-share', { percent })}><span style={{ '--share': `${percent}%` } as CSSProperties} /></div>
      </div>}
      <div className="machine-card__foot">
        {/* The actions wrap among themselves on a narrow card; the menu keeps its place at the end of the first line. */}
        <div className="machine-card__actions">
          {onStart && <Button size="sm" onClick={() => onStart(m.id)}><Icon name="plus" className="size-3.5" />{t('machine-start-here')}</Button>}
          {onNewTerminal && <Button size="sm" variant="ghost" onClick={() => onNewTerminal(m.id)}><Icon name="terminal" className="size-3.5" />{t('machine-new-terminal')}</Button>}
        </div>
        <Menu label={t('machine-more', { name: m.name })} items={[
          { label: t('machine-copy-id'), run: () => { void navigator.clipboard?.writeText(m.id).catch(() => {}); } },
          { label: t('machine-agent-settings'), run: () => { window.location.hash = '#/device/agents'; } },
        ]}><Icon name="more" /></Menu>
      </div>
    </> : <>
      {/* Offline is not finished: what ran there keeps running (nodes are autonomous) and reports back when the machine connects. */}
      {m.agentsRunning > 0 && <Callout tone="info" icon="clock" title={t('machine-away-agents', { count: m.agentsRunning })}>
        <span className="text-xs">{t('machine-away-explain', { name: isolated(m.name) })}</span>
      </Callout>}
      {onForget && <div className="machine-card__foot machine-card__foot--away">
        <span className="text-xs text-muted">{t('machine-not-back')}</span>
        <span className="flex-1" />
        <Button size="sm" variant="danger-ghost" onClick={onForget}><Icon name="trash" className="size-3.5" />{t('forget-machine-action')}</Button>
      </div>}
    </>}
  </article>;
}
