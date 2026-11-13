// Client-side view model. Components render this; rendering never mutates state (AGENTS.md: render is pure).

// ADR-0058: one mode or model an agent offers. Name and description are the agent's own words.
export interface SessionOptionModel { id: string; name: string; description: string; skipsPermissions: boolean }
// What an agent reported about its session: the modes and models it offers and the ones in use, how full its context is, and its own
// cost (in micro-units of `currency`). Absent parts were never reported.
export interface SessionModel {
  modes: SessionOptionModel[];
  models: SessionOptionModel[];
  currentMode: string;
  currentModel: string;
  contextTokens: number;
  contextWindow: number;
  costMicros?: number;
  currency?: string;
}

export type UiState = 'working' | 'needs-you' | 'finished' | 'idle' | 'unknown';

export interface AgentTileModel {
  id: string;
  name: string; // generated short name, e.g. "claude · amber"
  accent: string; // CSS color for this instance
  machine: string;
  machineId?: string; // Enrollment identity; host names are display labels and need not be unique.
  workspace: string;
  state: UiState;
  preview: string; // last lines of the terminal, from a cell snapshot
  since?: number; // when the current state began (ms since epoch); orders the attention flow
  tokens?: number; // tokens used so far (input + output); the estimated cost comes separately from the hub (`usage.report`, ADR-0033)
  reportedBy?: 'agent' | 'screen'; // where the state came from, shown by "why?"
  branch?: string; // the workspace's branch, when it has one
  terminal?: boolean; // false for a chat agent: it has no terminal, only prompts
  projectId?: string;
  shared?: boolean;
  git?: boolean;
  project?: string; // the project's folder name, to group agents by project
  workspaceRef?: { node: string; id: string; directory?: boolean }; // where to ask the node about this workspace (`git.diff`)
  filesChanged?: number; // files with uncommitted changes in the agent's workspace
  closed?: boolean; // its process is gone; the list keeps it until the user clears it
  exitCode?: number;
  startedMs?: number;
  endedMs?: number;
  session?: SessionModel; // ADR-0058, for agents whose adapter reported it
  parentId?: string; // ADR-0057: the agent that started this one (hub-wide key)
  parentName?: string;
  // ADR-0060: it was running when its machine stopped. `resumable`: it can continue its own conversation (Resume); otherwise only Start
  // again (a new session in the same folder).
  // `stillRunning`: its process from before the restart still runs; it must be stopped before it can be resumed.
  interrupted?: { resumable: boolean; stillRunning?: boolean };
}

// ADR-0060: what to do with an interrupted agent: continue its conversation, start a new one in its folder, or stop its process that
// outlived the restart.
export type ResumeAction = 'resume' | 'again' | 'stop';

// ADR-0057: each helper right after the agent that started it, with how deep it sits. A helper whose parent is not in `list` stays in
// its place at depth 0.
export function helperTree<T extends { id: string; parentId?: string }>(list: readonly T[]): { agent: T; depth: number }[] {
  const out: { agent: T; depth: number }[] = [];
  const seen = new Set<string>();
  const add = (a: T, depth: number) => {
    if (seen.has(a.id) || depth > 8) return;
    seen.add(a.id);
    out.push({ agent: a, depth });
    for (const c of list) if (c.parentId === a.id) add(c, depth + 1);
  };
  for (const a of list) if (!a.parentId || !list.some((p) => p.id === a.parentId)) add(a, 0);
  for (const a of list) if (!seen.has(a.id)) out.push({ agent: a, depth: 0 });
  return out;
}

// Protocol AgentState (schema/agora/v1/common.proto) → what the user sees.
export function uiState(s: number | undefined): UiState {
  switch (s) {
    case 1: return 'working';
    case 2: return 'needs-you';
    case 3: return 'idle';
    case 4: return 'finished';
    default: return 'unknown';
  }
}

// The attention strip lists everything that needs the user.
export function needsAttention(agents: readonly AgentTileModel[]): AgentTileModel[] {
  return agents.filter((a) => a.state === 'needs-you');
}

// Most urgent first: needs you, working, idle, finished, unknown; stable inside a group.
const ORDER: Record<UiState, number> = { 'needs-you': 0, working: 1, idle: 2, finished: 3, unknown: 4 };
export function sortFleet(agents: readonly AgentTileModel[]): AgentTileModel[] {
  return agents
    .map((a, i) => [a, i] as const)
    .sort(([a, i], [b, j]) => ORDER[a.state] - ORDER[b.state] || i - j)
    .map(([a]) => a);
}

// Attention flow ("next that needs me"): oldest waiting first, then the one after `currentId` in that order, so
// repeated presses cycle through everything that needs the user. `undefined` when nothing needs them.
export function nextNeedingMe(agents: readonly AgentTileModel[], currentId?: string): AgentTileModel | undefined {
  const waiting = needsAttention(agents)
    .map((a, i) => [a, i] as const)
    .sort(([a, i], [b, j]) => (a.since ?? Infinity) - (b.since ?? Infinity) || i - j)
    .map(([a]) => a);
  if (waiting.length === 0) return undefined;
  const at = waiting.findIndex((a) => a.id === currentId);
  return waiting[(at + 1) % waiting.length];
}

// Demo models may omit identity; every wire-backed agent has it.
export const machineKey = (agent: AgentTileModel): string => agent.machineId ?? agent.workspaceRef?.node ?? agent.machine;
