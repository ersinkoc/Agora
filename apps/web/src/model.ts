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
  skipsPermissions?: boolean; // ADR-0059: started with an option that lets it act without asking (the "does not ask" badge)
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

// What the node reports as an agent's exit code (crates/agora-node/src/daemon.rs, `AgentExited`):
//   0 or more     the program's own exit status;
//   -N            the program died of signal N (the node sends SIGTERM, SIGKILL and SIGHUP itself when it stops or restarts agents);
//   -1            it ended and neither a status nor a signal was known;
//   i32::MIN      stopped on request, or never known (the node re-announcing an agent it only remembers).
export const EXIT_UNKNOWN = -2147483648;
// Signals that mean the program itself crashed: illegal instruction, abort, bus error, arithmetic error, segmentation fault.
const CRASH_SIGNALS = new Set([4, 6, 7, 8, 11]);

// A closed agent that genuinely failed: its program exited with a non-zero status, or crashed. A stop (i32::MIN), a signal the node
// sends itself, an unknown status and an agent interrupted with its machine are not failures. Interim (no state token for it yet): it
// is drawn with a neutral cross and the words "Ended with an error", never as finished.
export function endedWithError(a: { closed?: boolean; exitCode?: number; interrupted?: unknown }): boolean {
  if (!a.closed || a.interrupted || a.exitCode === undefined) return false;
  const code = a.exitCode;
  return code > 0 || CRASH_SIGNALS.has(-code);
}

// The exit status worth showing: only a program's own non-zero status.
export const shownExitCode = (a: { exitCode?: number }): number | undefined => (a.exitCode !== undefined && a.exitCode > 0 ? a.exitCode : undefined);

// A machine's name, and its id as well only where two machines share that name (the id tells them apart, so it is shown when it is
// needed and not otherwise). `others` is every machine in the list, this one included.
export function sharesName(m: { id: string; name: string }, others: Iterable<{ id: string; name: string }>): boolean {
  for (const o of others) if (o.id !== m.id && o.name === m.name) return true;
  return false;
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
