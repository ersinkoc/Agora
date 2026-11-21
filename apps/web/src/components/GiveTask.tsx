import * as Dialog from '@radix-ui/react-dialog';
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import { sharesName, type AgentTileModel } from '../model';
import { actsFreelyNow, canTake, giveAsk, giveCandidates, useGiveSupport, type GiveResult, type GiveTask } from '../net/give';
import { OWNER_MAX, TITLE_MAX, plain } from './boardColumns';
import { NoAskBadge } from './NoAskBadge';
import { StateBadge } from './StateBadge';

// ADR-0095 step 2: "Give to an agent". Present only in a window signed in to a hub; the board's cards and the command palette read it to
// know whether to offer the action at all.
export interface GiveApi {
  agents: readonly AgentTileModel[];
  /** Machines connected now; an agent on another one is not offered. */
  online?: ReadonlySet<string>;
  machines: readonly { id: string; name: string }[];
  send: (agent: AgentTileModel, task: GiveTask) => Promise<GiveResult>;
}
export const GiveContext = createContext<GiveApi | undefined>(undefined);

/** Whether a Backlog card of `projectId` offers "Give to an agent" now: a hub window, and an agent of the project that may take it. */
export function useCanGive(projectId: string): boolean {
  const give = useContext(GiveContext);
  useGiveSupport();
  return !!give && giveCandidates(give.agents, projectId, give.online).length > 0;
}

// Names in the confirmation are shown in full (only control and direction characters are taken out); this bound only stops a hostile
// name from filling the screen.
const FULL = 256;

const machineName = (a: AgentTileModel, machines: readonly { id: string; name: string }[]) => {
  const m = { id: a.machineId ?? '', name: a.machine };
  return sharesName(m, machines) ? t('machine-name-id', { name: m.name, id: m.id }) : m.name;
};

/** "Acts freely" for an agent that acts without asking now: started so, or switched to a mode or setting that skips approvals. */
function FreeBadge({ agent }: { agent: AgentTileModel }) {
  return <NoAskBadge agent={{ skipsPermissions: actsFreelyNow(agent) }} session={!agent.skipsPermissions} compact nested />;
}

/** One agent in the picker: name, kind, machine, its state, and whether it acts without asking. One button, at least 44 px high. */
export function GiveRow({ agent, depth, machines, onPick }: { agent: AgentTileModel; depth: number; machines: readonly { id: string; name: string }[]; onPick: () => void }) {
  const indent = depth === 0 ? '' : depth === 1 ? 'ms-5' : 'ms-10';
  return (
    <li className={indent} data-agent-id={agent.id}>
      <button type="button" onClick={onPick} className="grid min-h-11 w-full min-w-0 gap-1 rounded-s border border-line bg-surface-0 px-3 py-2 text-start text-ink transition-colors duration-150 hover:bg-surface-2">
        <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <strong className="min-w-0 break-words text-sm">{plain(agent.name, OWNER_MAX)}</strong>
          <StateBadge state={agent.state} />
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
      <dl className="m-0 grid gap-2 rounded-s border border-line bg-surface-0 p-3">
        {row(t('give-field-task'), plain(task.title, TITLE_MAX) || t('board-task-untitled'))}
        {row(t('give-field-agent'), plain(agent.name, FULL))}
        {row(t('give-field-kind'), plain(agent.kind ?? '', FULL))}
        {row(t('give-field-machine'), plain(machineName(agent, machines), FULL))}
        {row(t('give-field-project'), plain(task.project, FULL))}
      </dl>
      {actsFreelyNow(agent) && (
        <p role="note" className="m-0 flex items-start gap-2 rounded-s border border-needs-you/60 p-3 text-sm text-ink">
          <FreeBadge agent={agent} />
          <span>{t('give-confirm-no-ask', { agent: plain(agent.name, FULL) })}</span>
        </p>
      )}
      <p className="muted m-0 text-sm">{t('give-confirm-body')}</p>
      <div className="flex flex-wrap justify-end gap-2">
        <button ref={back} type="button" className="button min-h-11" disabled={busy} onClick={onBack}>{t('give-back')}</button>
        <button type="button" className="button button--primary min-h-11" disabled={busy} onClick={onSend}>{busy ? t('give-sending') : t('give-send')}</button>
      </div>
    </div>
  );
}

/** What happened: given now, given and waiting for the turn to end, or why it was not. While the machine waits for the owner's permission
 *  (`grant`) it says so and shows the answer of the command the grant dialog sends again; choosing another agent is not offered then,
 *  since that would be another try while this one may still go through. */
export function GiveOutcome({ result, agent, onClose, onBack, onSettled }: { result: GiveResult; agent: AgentTileModel; onClose: () => void; onBack: () => void; onSettled?: (r: GiveResult) => void }) {
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
        ? <p role="status" className="m-0 text-sm text-ink">{t(result.queued ? 'give-queued' : 'give-sent', { agent: plain(agent.name, FULL) })}</p>
        : <p role={grant ? 'status' : 'alert'} className="m-0 text-sm text-ink">{t(result.key, result.args)}</p>}
      <div className="flex flex-wrap justify-end gap-2">
        {!result.ok && !grant && <button type="button" className="button min-h-11" onClick={onBack}>{t('give-other-agent')}</button>}
        <button ref={done} type="button" className="button button--primary min-h-11" onClick={onClose}>{t('give-done')}</button>
      </div>
    </div>
  );
}

// The confirmation keeps the agent's id, not its tile: what it shows (state, "acts freely") and what is checked at send come from the
// current data, so a change while it is open is shown and an agent that stopped or left the project is not sent to.
type Step = { at: 'pick' } | { at: 'confirm'; agentId: string; busy?: boolean } | { at: 'result'; agent: AgentTileModel; result: GiveResult };

/** The dialog for one task: pick an agent of the task's project, confirm, send. */
export function GiveDialog({ task, give, onClose }: { task: GiveTask; give: GiveApi; onClose: () => void }) {
  useGiveSupport();
  const [step, setStep] = useState<Step>({ at: 'pick' });
  const candidates = giveCandidates(give.agents, task.projectId, give.online);
  const latest = useRef(give);
  latest.current = give;
  const current = (id: string) => latest.current.agents.find((a) => a.id === id);
  const send = async (agentId: string) => {
    const agent = current(agentId);
    // Checked again at the moment of sending: it may have stopped, been closed or interrupted, or moved to another project meanwhile.
    if (!agent || !canTake(agent, task.projectId, latest.current.online)) {
      setStep({ at: 'result', agent: agent ?? ({ id: agentId, name: '', machine: '' } as AgentTileModel), result: { ok: false, key: 'give-target-gone' } });
      return;
    }
    setStep({ at: 'confirm', agentId, busy: true });
    const result = await latest.current.send(agent, task);
    setStep({ at: 'result', agent, result });
  };
  const chosen = step.at === 'confirm' ? current(step.agentId) : undefined;
  const title = step.at === 'confirm' && chosen ? t('give-confirm-title', { agent: plain(chosen.name, FULL) }) : t('give-title');
  // While the command is on its way the dialog stays: its answer (sent, queued, refused) is only shown here.
  const sending = step.at === 'confirm' && !!step.busy;
  const hold = (e: Event) => { if (sending) e.preventDefault(); };
  // A key whose press began before the dialog opened (the Enter that chose the palette entry) must not activate the row that has the
  // focus now: a browser clicks a button on Enter's keypress and on Space's keyup, so both are ignored until a key is pressed here.
  const armed = useRef(false);
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
          className="fixed left-1/2 top-1/2 z-50 grid max-h-[calc(100dvh-2rem)] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 gap-3 overflow-auto rounded-l border border-line bg-surface-1 p-5 shadow-card">
          <Dialog.Title className="m-0 break-words text-base font-semibold tracking-normal text-ink normal-case">{title}</Dialog.Title>
          {step.at === 'pick' && <>
            <p id="give-task-title" dir="auto" className="m-0 break-words rounded-s border border-line bg-surface-0 p-2.5 text-sm text-ink">{plain(task.title, TITLE_MAX) || t('board-task-untitled')}</p>
            {candidates.length === 0
              ? <p className="muted m-0 text-sm">{t('give-none', { project: task.project })}</p>
              : <>
                <p className="muted m-0 text-sm">{t('give-pick-hint', { project: task.project })}</p>
                <ul aria-label={t('give-pick-label')} className="m-0 grid list-none gap-2 p-0">
                  {candidates.map(({ agent, depth }) => <GiveRow key={agent.id} agent={agent} depth={depth} machines={give.machines} onPick={() => setStep({ at: 'confirm', agentId: agent.id })} />)}
                </ul>
              </>}
            <div className="flex justify-end"><Dialog.Close className="button min-h-11">{t('give-cancel')}</Dialog.Close></div>
          </>}
          {step.at === 'confirm' && (chosen && (step.busy || canTake(chosen, task.projectId, give.online))
            ? <GiveConfirm task={task} agent={chosen} machines={give.machines} busy={step.busy} onSend={() => void send(step.agentId)} onBack={() => setStep({ at: 'pick' })} />
            : <GiveOutcome result={{ ok: false, key: 'give-target-gone' }} agent={chosen ?? ({ id: step.agentId, name: '', machine: '' } as AgentTileModel)} onClose={onClose} onBack={() => setStep({ at: 'pick' })} />)}
          {step.at === 'result' && <GiveOutcome key={step.result.ok ? 'ok' : step.result.key} result={step.result} agent={step.agent} onClose={onClose} onBack={() => setStep({ at: 'pick' })}
            onSettled={(result) => setStep({ at: 'result', agent: step.agent, result })} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** The card's action: opens the dialog for this task. A quiet chip, so a column of cards is not a column of primary buttons; on a touch
 *  screen it grows to the tap size like every chip that acts (app.css). */
export function GiveButton({ task }: { task: GiveTask }) {
  return (
    <button type="button" className="ide-chip w-fit min-h-8 px-2.5 hover:border-accent hover:text-ink" aria-label={t('give-action-label', { title: plain(task.title, TITLE_MAX) || t('board-task-untitled') })}
      onClick={() => giveAsk.ask(task)}>
      {t('give-action')}
    </button>
  );
}
