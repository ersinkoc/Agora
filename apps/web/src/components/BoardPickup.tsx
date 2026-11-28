import { createContext, useContext, useEffect, useId, useState } from 'react';
import { t } from '../i18n';
import { visible } from '../net/hidden';
import type { PickupSettings } from '../net/pickup';
import { Toggle } from './kit';

/**
 * ADR-0131: what the owner's window offers for automatic pickup. `machines` are the machines that have the project, `kinds` the agent
 * kinds the window has seen in it; `save` and `mark` are left out while the link is not live. Given only to the owner's window.
 */
export interface BoardPickupApi {
  machines: (projectId: string) => readonly { id: string; name: string }[];
  kinds: (projectId: string) => readonly string[];
  read: (projectId: string) => Promise<PickupSettings>;
  save?: (projectId: string, next: Pick<PickupSettings, 'enabled' | 'nodeIds' | 'agentKinds'>) => Promise<PickupSettings>;
  mark?: (projectId: string, taskId: string, eligible: boolean) => Promise<void>;
}
export const BoardPickupContext = createContext<BoardPickupApi | undefined>(undefined);

const REASONS: Record<string, string> = { review_off: 'board-pickup-paused-review', not_shared: 'board-pickup-paused-shared' };

/** On a Review-enabled project's board card: whether idle agents take the tasks the owner marks, and which machines and kinds may. */
export function BoardPickup({ projectId }: { projectId: string }) {
  const api = useContext(BoardPickupContext);
  const id = useId();
  const [settings, setSettings] = useState<PickupSettings>();
  const [state, setState] = useState<'reading' | 'ready' | 'unread' | 'saving' | 'failed'>('reading');
  useEffect(() => {
    if (!api) return;
    let stop = false;
    setState('reading');
    setSettings(undefined);
    api.read(projectId).then((s) => { if (!stop) { setSettings(s); setState('ready'); } }).catch(() => { if (!stop) setState('unread'); });
    return () => { stop = true; };
  }, [api?.read, projectId]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!api || state === 'reading' || state === 'unread' || !settings) return null;
  const save = (next: Partial<Pick<PickupSettings, 'enabled' | 'nodeIds' | 'agentKinds'>>) => {
    if (!api.save) return;
    setState('saving');
    api.save(projectId, { enabled: settings.enabled, nodeIds: settings.nodeIds, agentKinds: settings.agentKinds, ...next })
      .then((s) => { setSettings(s); setState('ready'); }).catch(() => setState('failed'));
  };
  const locked = !api.save || state === 'saving';
  // What is saved is always offered, also a machine or kind the window does not see now: opening this never drops it.
  const machines = [...api.machines(projectId)];
  for (const n of settings.nodeIds) if (!machines.some((m) => m.id === n)) machines.push({ id: n, name: n });
  const kinds = [...new Set([...api.kinds(projectId), ...settings.agentKinds])].sort();
  const flip = (list: string[], item: string, on: boolean) => (on ? [...list, item] : list.filter((x) => x !== item));
  const paused = settings.enabled && !settings.active;
  return (
    <div className="grid min-w-0 gap-1.5 border-t border-line pt-2 text-xs" data-board-pickup>
      <div className="flex items-center justify-between gap-2">
        <span id={`${id}-label`} className="text-muted">{t('board-pickup-label')}</span>
        <Toggle checked={settings.enabled} labelledBy={`${id}-label`} disabled={locked} onChange={(enabled) => save({ enabled })} />
      </div>
      {settings.enabled && <>
        <fieldset className="m-0 grid min-w-0 gap-1 border-0 p-0" disabled={locked}>
          <legend className="mb-0.5 p-0 text-muted">{t('board-pickup-machines')}</legend>
          {machines.length === 0 && <p className="m-0 text-muted">{t('board-pickup-none')}</p>}
          {machines.map((m) => <label key={m.id} className="flex min-w-0 items-center gap-2">
            <input type="checkbox" checked={settings.nodeIds.includes(m.id)} onChange={(e) => save({ nodeIds: flip(settings.nodeIds, m.id, e.target.checked) })} />
            <span className="min-w-0 truncate">{visible(m.name)}</span>
          </label>)}
        </fieldset>
        <fieldset className="m-0 grid min-w-0 gap-1 border-0 p-0" disabled={locked}>
          <legend className="mb-0.5 p-0 text-muted">{t('board-pickup-kinds')}</legend>
          {kinds.length === 0 && <p className="m-0 text-muted">{t('board-pickup-none')}</p>}
          {kinds.map((k) => <label key={k} className="flex min-w-0 items-center gap-2">
            <input type="checkbox" checked={settings.agentKinds.includes(k)} onChange={(e) => save({ agentKinds: flip(settings.agentKinds, k, e.target.checked) })} />
            <span className="min-w-0 truncate font-mono">{visible(k)}</span>
          </label>)}
        </fieldset>
        <p className="m-0 text-muted">{t('board-pickup-hint')}</p>
      </>}
      {paused && <p className="m-0 text-muted">{t(REASONS[settings.inactiveReason] ?? 'board-pickup-paused')}</p>}
      {state === 'failed' && <p role="alert" className="m-0 text-danger">{t('board-pickup-failed')}</p>}
    </div>
  );
}

/** On an open task's card: the owner marks it for an idle agent, or clears the mark. Without a live link it only says the mark. */
export function PickupMark({ projectId, taskId, marked }: { projectId: string; taskId: string; marked: boolean }) {
  const api = useContext(BoardPickupContext);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  if (!api) return null;
  if (!api.mark) return marked ? <span className="text-xs text-muted">{t('board-pickup-marked')}</span> : null;
  const mark = api.mark;
  return <span className="flex min-w-0 flex-wrap items-center gap-2">
    {marked && <span className="text-xs text-muted">{t('board-pickup-marked')}</span>}
    <button type="button" className="kit-btn kit-btn--ghost kit-btn--sm w-fit" disabled={busy}
      onClick={() => { setBusy(true); setFailed(false); mark(projectId, taskId, !marked).catch(() => setFailed(true)).finally(() => setBusy(false)); }}>
      {t(marked ? 'board-pickup-unmark' : 'board-pickup-mark')}
    </button>
    {failed && <span role="alert" className="text-xs text-danger">{t('board-pickup-failed')}</span>}
  </span>;
}
