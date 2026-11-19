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
}
export const ChatActionsContext = createContext<ChatActions>({});

// The outcome of sending a prompt, when the app tells more than yes or no.
export interface PromptResult { ok: boolean; queued?: boolean; code?: string }
export type PromptSend = (agent: AgentTileModel, text: string) => Promise<boolean | PromptResult>;
