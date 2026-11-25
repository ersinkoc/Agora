import { createContext } from 'react';
import type { AgentTileModel } from '../model';
import type { ActionResult } from '../net/refusal';

// What a chat window can ask of an agent beyond a prompt (ADR-0090). A context, so the app provides it once and the conversation screen
// (in the console, the full-size view and the phone) reads it without each of them passing it down. Absent in the gallery and the demo.
export interface ChatActions {
  // `AgentSetConfig`: set one of the agent's own settings (model, mode, effort, ...). Resolves false when the agent or node refused.
  setConfig?: (agentKey: string, configId: string, value: string) => Promise<ActionResult>;
  // `AgentCancelTurn`: end the agent's current turn (the agent stays); `dropQueue` also drops the prompts waiting behind it.
  cancelTurn?: (agentKey: string, dropQueue: boolean) => Promise<ActionResult>;
  // Hub-wide keys of agents whose node answered a prompt in the rich form (`AgentPromptResult`): the node speaks `chat.rich`.
  rich?: ReadonlySet<string>;
  // Hub-wide keys `<node>/<approval id>` of the approvals that wait now: a control that sent a switch needing approval watches for its card to go.
  approvals?: ReadonlySet<string>;
}
export const ChatActionsContext = createContext<ChatActions>({});

// The outcome of sending a prompt, when the app tells more than yes or no.
export interface PromptResult { ok: boolean; queued?: boolean; code?: string }
/** Why a prompt did not go through, as the composer says it: too many waiting, the agent did not take it in time (ADR-0110), the agent
 *  stopped (ADR-0110), or anything else. */
export type PromptFailure = 'full' | 'no_answer' | 'exited' | 'failed';
export function promptFailure(code: string | undefined): PromptFailure {
  switch (code) {
    case 'agent.queue_full': return 'full';
    case 'adapter.no_answer': return 'no_answer';
    case 'adapter.agent_exited': return 'exited';
    default: return 'failed';
  }
}
export type PromptSend = (agent: AgentTileModel, text: string) => Promise<boolean | PromptResult>;
