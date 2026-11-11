// Client-side view model. Components render this; rendering never mutates state (AGENTS.md: render is pure).

export type UiState = 'working' | 'needs-you' | 'finished' | 'idle' | 'unknown';

export interface AgentTileModel {
  id: string;
  name: string; // generated short name, e.g. "claude · amber"
  accent: string; // CSS color for this instance
  machine: string;
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
