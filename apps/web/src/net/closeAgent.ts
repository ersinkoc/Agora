// Closing a running agent from the window (ADR-0053 lists it as closed afterwards). The command is `AgentStop`, the one `agora agent stop`
// sends; nothing here is a new message. Pure, so it is tested without a page.
import { useSyncExternalStore } from 'react';
import type { AgentTileModel, CommandModel } from '../model';

/** Why a close did not happen, as one of the web's own texts. A grant that is still needed is asked for by its own dialog. */
export function closeReasonKey(code: string): string {
  switch (code) {
    case 'auth.forbidden': return 'close-failed-forbidden';
    case 'node.policy_denied': return 'close-failed-policy';
    case 'approval.grant_required': return 'close-failed-grant';
    case 'approval.signature_invalid': return 'close-failed-trust';
    case 'node.offline': return 'close-failed-offline';
    case 'node.timeout': return 'close-failed-timeout';
    case 'agent.not_found': return 'close-failed-gone';
    case 'protocol.capability_missing': return 'close-failed-unsupported';
    case 'hub.unreachable': return 'close-failed-hub';
    default: return 'close-failed';
  }
}

const bare = (name: string): string => name.replace(/^\/+/, '').toLowerCase();

/** `/exit` or `/quit` alone in the box, when the agent offers no command of that name itself (then the agent's own command wins and is sent). */
export function isCloseCommand(text: string, offered: readonly CommandModel[]): boolean {
  const word = /^\/(exit|quit)$/i.exec(text.trim())?.[1]?.toLowerCase();
  return !!word && !offered.some((c) => bare(c.name) === word);
}

/** A turn is under way (or waits for an answer): closing loses it. */
export const losesWork = (a: Pick<AgentTileModel, 'state' | 'closed'>): boolean => !a.closed && (a.state === 'working' || a.state === 'needs-you');

// Which agent the confirm (a dialog over the window, #1070) is open for, and which item asked for it: `row` its row in the agent list,
// `pane` its own pane or screen. One at a time, so nothing else on the page can be closed by a click meant for this one: the id is the clicked item's.
export interface CloseAsk { id: string; where: 'row' | 'pane' }
let asking: CloseAsk | undefined;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());
export const closeAsk = {
  ask(id: string, where: CloseAsk['where']) { asking = { id, where }; emit(); },
  cancel() { if (asking) { asking = undefined; emit(); } },
  get: () => asking,
  subscribe: (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; },
};
export const useCloseAsk = (): CloseAsk | undefined => useSyncExternalStore(closeAsk.subscribe, closeAsk.get, closeAsk.get);
