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
