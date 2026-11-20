import { createContext, useContext, useEffect, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { closeAsk, closeReasonKey, losesWork, useCloseAsk, type CloseAsk } from '../net/closeAgent';
import { Menu } from '../layout/Menu';
import { Icon } from './Icon';

// What the window can do to end an agent: `AgentStop` for the agent it is given (undefined when it ended, else the error code). A context, so
// the console, the full-size view and the phone read it without passing it down. Absent without a hub (the gallery and the demo): no control.
export const CloseAgentContext = createContext<((agent: AgentTileModel) => Promise<string | undefined>) | undefined>(undefined);

/** Whether this agent can be closed from here: running, and the window can send the command. */
export const useCanClose = (agent: AgentTileModel): boolean => !!useContext(CloseAgentContext) && !agent.closed;

// The header's menu: "Close agent" opens the confirm in the agent's own pane. Not "Stop turn" (that one is in the composer and keeps the agent).
export function CloseAgentMenu({ agent, where = 'pane' }: { agent: AgentTileModel; where?: CloseAsk['where'] }) {
  const can = useCanClose(agent);
  if (!can) return null;
  return <Menu label={t('agent-actions', { name: agent.name })} items={[{ label: t('close-agent'), run: () => closeAsk.ask(agent.id, where) }]}><Icon name="more" /></Menu>;
}

// The inline confirm: what happens, a warning when a turn is under way, one button to do it and one to leave the agent running. Drawn only
// where it was asked for, for the agent that was asked for.
export function CloseConfirm({ agent, where }: { agent: AgentTileModel; where: CloseAsk['where'] }) {
  const close = useContext(CloseAgentContext);
  const asked = useCloseAsk();
  const mine = asked?.id === agent.id && asked.where === where;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  // Closed meanwhile (here, from another window, or after a permission was given): nothing is left to ask.
  useEffect(() => { if (agent.closed && asked?.id === agent.id) closeAsk.cancel(); }, [agent.closed, asked?.id, agent.id]);
  useEffect(() => { if (!mine) { setBusy(false); setError(undefined); } }, [mine]);
  if (!mine || !close || agent.closed) return null;
  const go = async () => {
    setBusy(true);
    setError(undefined);
    const code = await close(agent);
    setBusy(false);
    if (code) setError(code); else closeAsk.cancel();
  };
  return (
    <div role="alertdialog" aria-label={t('close-title', { name: agent.name })} data-close-confirm className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
      <div className="min-w-0 flex-1 basis-64 text-xs">
        <strong className="block">{t('close-title', { name: agent.name })}</strong>
        <span className="text-muted">{t('close-body')}</span>
        {losesWork(agent) && <span className="mt-0.5 block text-needs-you" data-close-warning>{t('close-working')}</span>}
        {error && <span role="alert" className="mt-0.5 block text-needs-you">{t('close-failed-prefix', { reason: t(closeReasonKey(error)) })}</span>}
      </div>
      <button type="button" className="button gap-1.5 border-removed px-3 py-1 text-xs text-removed" disabled={busy} onClick={() => void go()}><Icon name="close" className="size-3.5" />{t('close-agent')}</button>
      <button type="button" className="button button--primary px-3 py-1 text-xs" onClick={() => closeAsk.cancel()}>{t('close-keep')}</button>
    </div>
  );
}
