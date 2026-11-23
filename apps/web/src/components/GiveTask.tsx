import * as Dialog from '@radix-ui/react-dialog';
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import { sharesName, type AgentTileModel } from '../model';
import { actsFreelyNow, canStartFor, canTake, cannotTakeKey, giveAsk, giveCandidates, offersGive, useGiveSupport, type GiveResult, type GiveTask } from '../net/give';
import { OWNER_MAX, TITLE_MAX, plain } from './boardColumns';
import type { DirectoryInfo, DirectoryListing } from '../net/directory';
import type { MachineModel, ProjectChoice } from '../net/view';
import { Icon } from './Icon';
import { NoAskBadge } from './NoAskBadge';
import { StartAgent, type KindChoice } from './StartAgent';
import { StatusPill } from './kit';

/** What the ordinary start window needs (ADR-0095 step 3, "start a new one"): the same choices, checks and signed command as "Start an
 *  agent", with `launch` answering the new agent's id (on its machine) or the refusal's locale key. Nothing here starts an agent another way. */
export interface StartForTask {
  projects: readonly ProjectChoice[];
  machines: readonly MachineModel[];
  trust?: { id: string; key: string };
  askApproval?: { can: (node: string) => boolean; ask: () => Promise<boolean> };
  loadKinds: (node: string) => Promise<KindChoice[]>;
  inspect: (node: string, path: string, createIdentity: boolean) => Promise<DirectoryInfo>;
  list?: (node: string, path: string) => Promise<DirectoryListing>;
  launch: (node: string, info: DirectoryInfo, kind: string, separate: boolean, acceptIdentityChange: boolean, launch: { ids: string[]; confirmed: string[]; settings: [string, string][] }) => Promise<{ agentId: string } | { failure: string }>;
}

// ADR-0095 step 2: "Give to an agent". Present only in a window signed in to a hub; the board's cards and the command palette read it to
// know whether to offer the action at all.
export interface GiveApi {
  agents: readonly AgentTileModel[];
  /** Machines connected now; an agent on another one is not offered. */
  online?: ReadonlySet<string>;
  machines: readonly { id: string; name: string }[];
  send: (agent: AgentTileModel, task: GiveTask) => Promise<GiveResult>;
  /** Starting a new agent for the task (a window that can start agents). */
  start?: StartForTask;
}
export const GiveContext = createContext<GiveApi | undefined>(undefined);

/** Whether a Backlog card of `projectId` offers "Give to an agent" now: a hub window, and an agent of the project that may take it or a
 *  window that can start one for it. */
export function useCanGive(projectId: string): boolean {
  const give = useContext(GiveContext);
  useGiveSupport();
  return !!give && !!projectId && ((!!give.start && canStartFor(give.start.projects, give.start.machines, projectId)) || giveCandidates(give.agents, projectId, give.online).length > 0);
}

// Names in the confirmation are shown in full (only control and direction characters are taken out); this bound only stops a hostile
// name from filling the screen.
const FULL = 256;

const machineName = (a: AgentTileModel, machines: readonly { id: string; name: string }[]) => {
  const m = { id: a.machineId ?? '', name: a.machine };
  return sharesName(m, machines) ? t('machine-name-id', { name: m.name, id: m.id }) : m.name;
};

/** The acts-without-asking sign for an agent that acts without asking now: started so, or switched to a mode or setting that skips approvals. */
function FreeBadge({ agent, nested = true }: { agent: AgentTileModel; nested?: boolean }) {
  return <NoAskBadge agent={{ skipsPermissions: actsFreelyNow(agent) }} session={!agent.skipsPermissions} nested={nested} />;
}

/** One agent in the picker: name, kind, machine, its state, and whether it acts without asking. One button, at least 44 px high. */
export function GiveRow({ agent, depth, machines, onPick }: { agent: AgentTileModel; depth: number; machines: readonly { id: string; name: string }[]; onPick: () => void }) {
  const indent = depth === 0 ? '' : depth === 1 ? 'ms-5' : 'ms-10';
  return (
    <li className={indent} data-agent-id={agent.id}>
      <button type="button" onClick={onPick} className="grid min-h-11 w-full min-w-0 gap-1 rounded-m border border-line bg-surface-1 px-3 py-2 text-start text-ink transition-colors duration-150 hover:border-line-strong hover:bg-surface-2">
        <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <strong className="min-w-0 break-words text-sm">{plain(agent.name, OWNER_MAX)}</strong>
          <StatusPill state={agent.state} />
          <FreeBadge agent={agent} />
        </span>
        <span className="min-w-0 break-words text-xs text-muted">
          {depth > 0 && agent.parentId ? t('give-row-helper', { kind: plain(agent.kind ?? '', OWNER_MAX), machine: plain(machineName(agent, machines), FULL) }) : t('give-row-detail', { kind: plain(agent.kind ?? '', OWNER_MAX), machine: plain(machineName(agent, machines), FULL) })}
        </span>
      </button>
    </li>
  );
}

/** The confirmation: agent, kind, machine and project in full, and that the agent acts without asking when it does. The focus goes to
 *  Back, never to "Give the task": a key held down since the palette or the picker must not send it (repeats are ignored as well). */
export function GiveConfirm({ task, agent, machines, busy, onSend, onBack }: { task: GiveTask; agent: AgentTileModel; machines: readonly { id: string; name: string }[]; busy?: boolean; onSend: () => void; onBack: () => void }) {
  const back = useRef<HTMLButtonElement>(null);
  useEffect(() => { back.current?.focus(); }, []);
  const row = (label: string, value: ReactNode, ltr = false) => (
    <div className="grid min-w-0 gap-0.5 sm:grid-cols-[7rem_minmax(0,1fr)] sm:gap-3">
      <dt className="text-xs text-muted">{label}</dt>
      <dd dir={ltr ? 'ltr' : 'auto'} className="m-0 min-w-0 break-words text-sm text-ink">{value}</dd>
    </div>
  );
  return (
    <div className="grid gap-3" data-give-step="confirm">
      <dl className="m-0 grid gap-2 rounded-m border border-line bg-surface-1 p-3">
        {row(t('give-field-task'), plain(task.title, TITLE_MAX) || t('board-task-untitled'))}
        {row(t('give-field-agent'), plain(agent.name, FULL))}
        {row(t('give-field-kind'), plain(agent.kind ?? '', FULL))}
        {row(t('give-field-machine'), plain(machineName(agent, machines), FULL))}
        {row(t('give-field-project'), plain(task.project, FULL))}
      </dl>
      {actsFreelyNow(agent) && (
        <p role="note" className="m-0 flex items-start gap-2 rounded-s border border-danger/60 p-3 text-sm text-ink">
          <FreeBadge agent={agent} nested={false} />
          <span>{t('give-confirm-no-ask', { agent: plain(agent.name, FULL) })}</span>
        </p>
      )}
      <p className="muted m-0 text-sm">{t('give-confirm-body')}</p>
      <div className="flex flex-wrap justify-end gap-2">
        <button ref={back} type="button" className="kit-btn min-h-11" disabled={busy} onClick={onBack}>{t('give-back')}</button>
        <button type="button" className="kit-btn kit-btn--primary min-h-11" disabled={busy} onClick={onSend}>{busy ? t('give-sending') : t('give-send')}</button>
      </div>
    </div>
  );
}

/** What happened: given now, given and waiting for the turn to end, or why it was not. While the machine waits for the owner's permission
 *  (`grant`) it says so and shows the answer of the command the grant dialog sends again; choosing another agent is not offered then,
 *  since that would be another try while this one may still go through. */
export function GiveOutcome({ result, agent, fresh, onClose, onBack, onSettled }: { result: GiveResult; agent: AgentTileModel; fresh?: boolean; onClose: () => void; onBack: () => void; onSettled?: (r: GiveResult) => void }) {
  const done = useRef<HTMLButtonElement>(null);
  useEffect(() => { done.current?.focus(); }, []);
  const grant = !result.ok ? result.grant : undefined;
  useEffect(() => {
    if (!grant) return;
    let live = true;
    void grant.then((r) => { if (live) onSettled?.(r); });
    return () => { live = false; };
  }, [grant]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="grid gap-3" data-give-step="result">
      {result.ok
        ? <p role="status" className="m-0 text-sm text-ink">{t(result.queued ? (fresh ? 'give-queued-new' : 'give-queued') : 'give-sent', { agent: plain(agent.name, FULL) })}</p>
        : <p role={grant ? 'status' : 'alert'} className="m-0 text-sm text-ink">{t(result.key, result.args)}</p>}
      <div className="flex flex-wrap justify-end gap-2">
        {!result.ok && !grant && <button type="button" className="kit-btn min-h-11" onClick={onBack}>{t('give-other-agent')}</button>}
        <button ref={done} type="button" className="kit-btn kit-btn--primary min-h-11" onClick={onClose}>{t('give-done')}</button>
      </div>
    </div>
  );
}

// The confirmation keeps the agent's id, not its tile: what it shows (state, "acts freely") and what is checked at send come from the
// current data, so a change while it is open is shown and an agent that stopped or left the project is not sent to.
// `start`: the ordinary start window, opened for this task; `refusal` is the last thing it refused, shown in the picker when the person
// comes back. `fresh`: the agent was just started from here (it may not be in the live view yet, and may still be starting).
type Step = { at: 'pick'; refusal?: string } | { at: 'start'; refusal?: string } | { at: 'confirm'; agentId: string; busy?: boolean; fresh?: boolean }
  | { at: 'result'; agent: AgentTileModel; result: GiveResult; fresh?: boolean };

// How long the window waits for a started agent to show up before it says so (the binding to that agent stays: it still follows).
export const APPEAR_WAIT_MS = 60_000;

/** Started, and not in the live view yet. After `APPEAR_WAIT_MS` it says the agent has not appeared yet; Back returns to the picker. */
function WaitingForAgent({ onBack }: { onBack: () => void }) {
  const [late, setLate] = useState(false);
  useEffect(() => { const id = setTimeout(() => setLate(true), APPEAR_WAIT_MS); return () => clearTimeout(id); }, []);
  return (
    <div className="grid gap-3" data-give-step="waiting">
      <p role="status" className="m-0 text-sm text-ink">{t(late ? 'give-start-late' : 'give-start-waiting')}</p>
      <div className="flex justify-end"><button type="button" className="kit-btn min-h-11" onClick={onBack}>{t('give-back')}</button></div>
    </div>
  );
}

/** "Start a new agent for this task": the picker's main action when no agent can take the task, else a quieter one under the list. */
function StartForTaskButton({ main, onClick }: { main: boolean; onClick: () => void }) {
  return (
    <button type="button" data-give-start="" onClick={onClick}
      className={main ? 'kit-btn kit-btn--primary min-h-11 justify-self-start' : 'kit-btn min-h-11 w-full justify-start border-dashed'}>
      <Icon name="plus" className="size-4" />{t('give-start-new')}
    </button>
  );
}

/** The dialog for one task: pick an agent of the task's project, confirm, send. */
export function GiveDialog({ task, give, onClose }: { task: GiveTask; give: GiveApi; onClose: () => void }) {
  useGiveSupport();
  const [step, setStep] = useState<Step>({ at: 'pick' });
  const candidates = giveCandidates(give.agents, task.projectId, give.online);
  const latest = useRef(give);
  latest.current = give;
  // A key whose press began before the dialog opened (the Enter that chose the palette entry) must not activate the row that has the
  // focus now: a browser clicks a button on Enter's keypress and on Space's keyup, so both are ignored until a key is pressed here.
  const armed = useRef(false);
  const current = (id: string) => latest.current.agents.find((a) => a.id === id);
  const send = async (agentId: string, fresh?: boolean) => {
    const agent = current(agentId);
    // Checked again at the moment of sending: it may have stopped, been closed or interrupted, or moved to another project meanwhile.
    if (!agent || !canTake(agent, task.projectId, latest.current.online)) {
      setStep({ at: 'result', agent: agent ?? ({ id: agentId, name: '', machine: '' } as AgentTileModel), result: { ok: false, key: cannotTakeKey(agent, latest.current.online) } });
      return;
    }
    setStep({ at: 'confirm', agentId, busy: true, fresh });
    const result = await latest.current.send(agent, task);
    setStep({ at: 'result', agent, result, fresh });
  };
  // The ordinary start: what it refused stays in its own window (with the trust command and the rest), and is shown again in the picker
  // when the person goes back. Once it started, the same confirmation follows for the new agent; nothing is sent before it.
  const starter = give.start;
  // Only where an agent started could be given the task: a connected machine with a folder of the project, not heard to lack tasks.
  const startable = !!starter && canStartFor(starter.projects, starter.machines, task.projectId);
  if (step.at === 'start' && starter) {
    const projects = starter.projects.filter((p) => p.projectId === task.projectId && offersGive(p.node));
    return <StartAgent projects={projects} machines={starter.machines.filter((m) => offersGive(m.id))} trust={starter.trust} askApproval={starter.askApproval} loadKinds={starter.loadKinds} inspect={starter.inspect} list={starter.list}
      forTask={{ projectId: task.projectId, project: plain(task.project, FULL), title: plain(task.title, TITLE_MAX) || t('board-task-untitled') }}
      onStart={async (node, info, kind, separate, acceptIdentityChange, launch) => {
        const r = await starter.launch(node, info, kind, separate, acceptIdentityChange, launch);
        if ('failure' in r) { setStep({ at: 'start', refusal: r.failure }); return r.failure; }
        // An older machine that does not say which agent it started: back to the picker, where the new agent is listed once it shows up.
        setStep(r.agentId ? { at: 'confirm', agentId: `${node}/${r.agentId}`, fresh: true } : { at: 'pick', refusal: 'give-start-unnamed' });
        return undefined;
      }}
      // The key that closed the start window must not also activate what has the focus in the picker.
      onClose={() => { armed.current = false; setStep((s) => ({ at: 'pick', refusal: s.at === 'start' ? s.refusal : undefined })); }} />;
  }
  const chosen = step.at === 'confirm' ? current(step.agentId) : undefined;
  const title = step.at === 'confirm' && chosen ? t('give-confirm-title', { agent: plain(chosen.name, FULL) }) : t('give-title');
  // While the command is on its way the dialog stays: its answer (sent, queued, refused) is only shown here.
  const sending = step.at === 'confirm' && !!step.busy;
  const hold = (e: Event) => { if (sending) e.preventDefault(); };
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o && !sending) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={step.at === 'pick' ? 'give-task-title' : undefined} data-give-dialog="" aria-busy={sending || undefined}
          onEscapeKeyDown={hold} onPointerDownOutside={hold} onInteractOutside={hold}
          // A held key repeats: only a fresh press activates a row or a button here, so holding Enter from the palette cannot pick and send.
          onKeyDownCapture={(e) => {
            if (!e.repeat) armed.current = true;
            else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); }
          }}
          onKeyPressCapture={(e) => { if (!armed.current) { e.preventDefault(); e.stopPropagation(); } }}
          onKeyUpCapture={(e) => { if (!armed.current && e.key === ' ') { e.preventDefault(); e.stopPropagation(); } }}
          onKeyDown={(e) => { if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1) e.stopPropagation(); }}
          className="fixed left-1/2 top-1/2 z-50 grid max-h-[calc(100dvh-2rem)] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 gap-3 overflow-auto rounded-l border border-line bg-surface-raised p-5 shadow-pop">
          {/* The kit's dialog header: an icon chip and the title. */}
          <div className="flex min-w-0 items-center gap-3"><span aria-hidden className="kit-chip kit-tone-accent"><Icon name="board" className="size-4" /></span>
            <Dialog.Title className="m-0 min-w-0 break-words text-base font-semibold tracking-normal text-ink normal-case">{title}</Dialog.Title></div>
          {step.at === 'pick' && <>
            <p id="give-task-title" dir="auto" className="m-0 break-words rounded-s border border-line bg-surface-0 p-2.5 text-sm text-ink">{plain(task.title, TITLE_MAX) || t('board-task-untitled')}</p>
            {step.refusal && <p role="alert" data-give-refusal="" className="m-0 rounded-s border border-danger/60 p-2.5 text-sm text-ink">{t(step.refusal)}</p>}
            {candidates.length === 0
              ? <>
                <p className="muted m-0 text-sm">{t(startable ? 'give-none-start' : starter ? 'give-none-plain' : 'give-none', { project: plain(task.project, FULL) })}</p>
                {startable && <StartForTaskButton main onClick={() => setStep({ at: 'start' })} />}
              </>
              : <>
                <p className="muted m-0 text-sm">{t('give-pick-hint', { project: plain(task.project, FULL) })}</p>
                <ul aria-label={t('give-pick-label')} className="m-0 grid list-none gap-2 p-0">
                  {candidates.map(({ agent, depth }) => <GiveRow key={agent.id} agent={agent} depth={depth} machines={give.machines} onPick={() => setStep({ at: 'confirm', agentId: agent.id })} />)}
                </ul>
                {startable && <StartForTaskButton main={false} onClick={() => setStep({ at: 'start' })} />}
              </>}
            <div className="flex justify-end"><Dialog.Close className="kit-btn min-h-11">{t('give-cancel')}</Dialog.Close></div>
          </>}
          {step.at === 'confirm' && (!chosen && step.fresh
            // Started, and not in the live view yet: the confirmation follows as soon as it shows up.
            ? <WaitingForAgent onBack={() => setStep({ at: 'pick' })} />
            : chosen && (step.busy || canTake(chosen, task.projectId, give.online))
            ? <GiveConfirm task={task} agent={chosen} machines={give.machines} busy={step.busy} onSend={() => void send(step.agentId, step.fresh)} onBack={() => setStep({ at: 'pick' })} />
            : <GiveOutcome result={{ ok: false, key: cannotTakeKey(chosen, give.online) }} agent={chosen ?? ({ id: step.agentId, name: '', machine: '' } as AgentTileModel)} onClose={onClose} onBack={() => setStep({ at: 'pick' })} />)}
          {step.at === 'result' && <GiveOutcome key={step.result.ok ? 'ok' : step.result.key} result={step.result} agent={step.agent} fresh={step.fresh} onClose={onClose} onBack={() => setStep({ at: 'pick' })}
            onSettled={(result) => setStep({ at: 'result', agent: step.agent, result, fresh: step.fresh })} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** The card's action: opens the dialog for this task. A quiet chip, so a column of cards is not a column of primary buttons; on a touch
 *  screen it grows to the tap size like every chip that acts (app.css). */
export function GiveButton({ task }: { task: GiveTask }) {
  return (
    <button type="button" className="kit-btn kit-btn--ghost kit-btn--sm w-fit -ms-1.5 text-accent" aria-label={t('give-action-label', { title: plain(task.title, TITLE_MAX) || t('board-task-untitled') })}
      onClick={() => giveAsk.ask(task)}>
      {t('give-action')}
    </button>
  );
}
