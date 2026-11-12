// The client's copy of the hub's view (crates/agora-hub/src/projection.rs): built from a Snapshot, then advanced by
// positioned events. Keys are hub-wide `<producer>/<local id>`. Pure: no sockets, no React.

import { agora } from '../gen/agora.js';
import { type AgentTileModel, uiState } from '../model';

type Ev = agora.v1.Event.$Properties;
type Num = number | bigint | { toString(): string } | null | undefined;

// uint64 fields arrive as number, Long or bigint depending on the generator; ids and times stay below 2^53.
export function num(x: Num): number {
  if (x === null || x === undefined) return 0;
  return typeof x === 'number' ? x : Number(x.toString());
}

export interface NodeV { hostname: string; os: string; online: boolean; lastEventMs: number; agentsRunning: number; memoryBytes?: number }
export interface WorkspaceV { nodeId: string; projectId: string; path: string; gitRoot: string; branch: string; managed: boolean; filesChanged: number }
export interface WorktreeV { checkoutId: string; path: string; branch: string; filesChanged: number }
export interface AgentV {
  nodeId: string; worktreeId: string; kind: string; state: number; reported: boolean; running: boolean;
  sinceMs: number;
  tokens: number; // input + output so far
}
export interface CheckoutV { nodeId: string; projectId: string; path: string }
export interface ApprovalV { agentId: string; action: string; detail: string; expiresAtMs: number }

export interface TaskV { projectId: string; title: string; ownerId: string; done: boolean }
export interface LeaseV { agentId: string; path: string; expiresAtMs: number }
export interface MessageV { from: string; to: string; body: string; atMs: number }

export interface View {
  pos: number;
  workspaces: Map<string, WorkspaceV>;
  agentWorkspaces: Map<string, string>;
  messages: MessageV[]; // the latest agent-to-agent messages, oldest first
  nodes: Map<string, NodeV>;
  checkouts: Map<string, CheckoutV>;
  worktrees: Map<string, WorktreeV>;
  agents: Map<string, AgentV>;
  approvals: Map<string, ApprovalV>;
  tasks: Map<string, TaskV>; // the shared board's tasks seen since this window opened (ADR-0039)
  leases: Map<string, LeaseV>;
  withTerminal: Set<string>; // agents that have a terminal; the others (chat agents) are talked to with prompts
}

const MAX_MESSAGES = 50;

export function emptyView(): View {
  return { pos: 0, workspaces: new Map(), agentWorkspaces: new Map(), messages: [], nodes: new Map(), checkouts: new Map(), worktrees: new Map(), agents: new Map(), approvals: new Map(), tasks: new Map(), leases: new Map(), withTerminal: new Set() };
}

export function fromSnapshot(s: agora.v1.Snapshot.$Properties): View {
  const v = emptyView();
  v.pos = num(s.pos);
  for (const w of s.workspaces ?? []) v.workspaces.set(w.workspaceId ?? '', { nodeId: w.nodeId ?? '', projectId: w.projectId ?? '', path: w.path ?? '', gitRoot: w.gitRoot ?? '', branch: w.branch ?? '', managed: !!w.managedWorktree, filesChanged: w.filesChanged ?? 0 });
  for (const a of s.agentWorkspaces ?? []) v.agentWorkspaces.set(a.agentId ?? '', a.workspaceId ?? '');
  for (const n of s.nodes ?? []) {
    v.nodes.set(n.nodeId ?? '', {
      hostname: n.hostname ?? '', os: n.os ?? '', online: !!n.online, lastEventMs: num(n.lastEventMs),
      agentsRunning: n.agentsRunning ?? 0,
      memoryBytes: Number(n.memoryUsedBytes ?? 0),
    });
  }
  for (const c of s.checkouts ?? []) {
    v.checkouts.set(c.checkoutId ?? '', { nodeId: c.nodeId ?? '', projectId: c.projectId ?? '', path: c.path ?? '' });
  }
  for (const w of s.worktrees ?? []) {
    v.worktrees.set(w.worktreeId ?? '', {
      checkoutId: w.checkoutId ?? '', path: w.path ?? '', branch: w.branch ?? '', filesChanged: w.filesChanged ?? 0,
    });
  }
  for (const a of s.agents ?? []) {
    v.agents.set(a.agentId ?? '', {
      nodeId: a.nodeId ?? '', worktreeId: a.worktreeId ?? '', kind: a.kind ?? '', state: a.state ?? 0,
      reported: !!a.reported, running: !!a.running, sinceMs: 0,
      tokens: num(a.inputTokens) + num(a.outputTokens),
    });
  }
  for (const tm of s.terminals ?? []) if (tm.agentId) v.withTerminal.add(tm.agentId);
  for (const t of s.tasks ?? []) {
    v.tasks.set(`${t.projectId ?? ''}/${t.taskId ?? ''}`, { projectId: t.projectId ?? '', title: t.title ?? '', ownerId: t.claimedByAgentId ?? '', done: !!t.done });
  }
  for (const l of s.leases ?? []) {
    v.leases.set(`${l.agentId}\u0000${l.path}`, { agentId: l.agentId ?? '', path: l.path ?? '', expiresAtMs: num(l.expiresAtMs) });
  }
  for (const p of s.approvals ?? []) {
    v.approvals.set(p.approvalId ?? '', {
      agentId: p.agentId ?? '', action: p.action ?? '', detail: p.detail ?? '', expiresAtMs: num(p.expiresAtMs),
    });
  }
  return v;
}

// Applies one event at log position `pos`. An event at or before the view's position was already folded in
// (a resume replays the tail), so it is ignored: applying is idempotent.
export function applyEvent(v: View, pos: number, ev: Ev): void {
  if (pos <= v.pos) return;
  v.pos = pos;
  const producer = ev.producerId ?? '';
  const q = (id: string | null | undefined) => `${producer}/${id ?? ''}`;
  // The hub's own events (the shared board) already carry `<node>/<agent>` ids.
  const qb = (id: string | null | undefined) => (producer === 'hub' ? (id ?? '') : q(id));
  const at = num(ev.atMs);
  const node = v.nodes.get(producer);
  if (node) node.lastEventMs = Math.max(node.lastEventMs, at);
  const e = ev as Record<string, any>;
  switch (ev.body) {
    case 'workspaceRegistered': { const w = e.workspaceRegistered; v.workspaces.set(q(w.workspaceId), { nodeId: producer, projectId: w.projectId ?? '', path: w.path ?? '', gitRoot: w.gitRoot ?? '', branch: w.branch ?? '', managed: !!w.managedWorktree, filesChanged: w.filesChanged ?? 0 }); break; }
    case 'agentWorkspaceBound': v.agentWorkspaces.set(q(e.agentWorkspaceBound.agentId), q(e.agentWorkspaceBound.workspaceId)); break;
    case 'workspaceStatusChanged': { const w = v.workspaces.get(q(e.workspaceStatusChanged.workspaceId)); if (w) w.filesChanged = e.workspaceStatusChanged.filesChanged ?? 0; break; }
    case 'nodeOnline': {
      const b = e.nodeOnline;
      // The node's process started: what it ran before is gone, and its waiting approvals are lost.
      for (const a of v.agents.values()) if (a.nodeId === b.nodeId && a.running) { a.running = false; a.state = 4 /* done */; }
      for (const k of [...v.approvals.keys()]) if (k.startsWith(`${b.nodeId}/`)) v.approvals.delete(k);
      for (const k of [...v.withTerminal]) if (k.startsWith(`${b.nodeId}/`)) v.withTerminal.delete(k);
      v.nodes.set(b.nodeId, { hostname: b.hostname ?? '', os: b.os ?? '', online: true, lastEventMs: at, agentsRunning: v.nodes.get(b.nodeId)?.agentsRunning ?? 0 });
      break;
    }
    case 'nodeOffline': {
      const n = v.nodes.get(e.nodeOffline.nodeId);
      if (n) n.online = false;
      break;
    }
    case 'checkoutDiscovered': {
      const b = e.checkoutDiscovered;
      v.checkouts.set(q(b.checkoutId), { nodeId: b.nodeId ?? '', projectId: b.projectId ?? '', path: b.path ?? '' });
      break;
    }
    case 'nodeResources': {
      const n = v.nodes.get(e.nodeResources.nodeId);
      if (n) { n.agentsRunning = e.nodeResources.agentsRunning ?? 0; n.memoryBytes = Number(e.nodeResources.memoryUsedBytes ?? 0); }
      break;
    }
    case 'worktreeCreated': {
      const b = e.worktreeCreated;
      v.worktrees.set(q(b.worktreeId), { checkoutId: q(b.checkoutId), path: b.path ?? '', branch: b.branch ?? '', filesChanged: 0 });
      break;
    }
    case 'worktreeRemoved':
      v.worktrees.delete(q(e.worktreeRemoved.worktreeId));
      break;
    case 'gitStatusChanged': {
      const w = v.worktrees.get(q(e.gitStatusChanged.worktreeId));
      if (w) w.filesChanged = e.gitStatusChanged.filesChanged ?? 0;
      break;
    }
    case 'agentStarted': {
      const b = e.agentStarted;
      if (b.worktreeId) v.agentWorkspaces.delete(q(b.agentId));
      // A new run of an id: whether it has a terminal is told again by the `terminalOpened` that follows (or not at all).
      v.withTerminal.delete(q(b.agentId));
      v.agents.set(q(b.agentId), {
        nodeId: b.nodeId ?? '', worktreeId: q(b.worktreeId), kind: b.kind ?? '', state: 3 /* idle */,
        reported: false, running: true, sinceMs: at, tokens: 0,
      });
      break;
    }
    case 'agentStateChanged': {
      const a = v.agents.get(q(e.agentStateChanged.agentId));
      if (a) {
        a.state = e.agentStateChanged.state ?? 0;
        a.reported = !!e.agentStateChanged.reported;
        a.sinceMs = at;
      }
      break;
    }
    case 'agentUsageReported': {
      const a = v.agents.get(q(e.agentUsageReported.agentId));
      if (a) a.tokens += num(e.agentUsageReported.inputTokens) + num(e.agentUsageReported.outputTokens);
      break;
    }
    case 'agentExited': {
      const a = v.agents.get(q(e.agentExited.agentId));
      if (a) { a.running = false; a.state = 4 /* done */; a.sinceMs = at; }
      break;
    }
    case 'terminalOpened':
      if (e.terminalOpened.agentId) v.withTerminal.add(q(e.terminalOpened.agentId));
      break;
    case 'approvalRequested': {
      const b = e.approvalRequested;
      v.approvals.set(q(b.approvalId), { agentId: q(b.agentId), action: b.action ?? '', detail: b.detail ?? '', expiresAtMs: num(b.expiresAtMs) });
      break;
    }
    case 'approvalResolved':
      v.approvals.delete(q(e.approvalResolved.approvalId));
      break;
    case 'messagePosted': {
      const b = e.messagePosted;
      const to = String(b.to ?? '');
      v.messages.push({ from: q(b.fromAgentId), to: to.includes('/') || to.startsWith('#') ? to : q(to), body: b.body ?? '', atMs: at });
      if (v.messages.length > MAX_MESSAGES) v.messages.shift();
      break;
    }
    // The hub writes board events with ids that are already `<node>/<agent>`.
    case 'taskChanged': {
      const b = e.taskChanged;
      v.tasks.set(`${b.projectId ?? ''}/${b.taskId ?? ''}`, { projectId: b.projectId ?? '', title: b.title ?? '', ownerId: b.claimedByAgentId ? qb(b.claimedByAgentId) : '', done: !!b.done });
      break;
    }
    case 'leaseGranted': {
      const b = e.leaseGranted;
      v.leases.set(`${qb(b.agentId)}\u0000${b.path}`, { agentId: qb(b.agentId), path: b.path ?? '', expiresAtMs: num(b.expiresAtMs) });
      break;
    }
    case 'leaseReleased':
      v.leases.delete(`${qb(e.leaseReleased.agentId)}\u0000${e.leaseReleased.path}`);
      break;
    default:
      break; // terminals, usage and files do not change what the fleet screen shows
  }
}

const ACCENTS = ['#ffb454', '#4fd1c5', '#a78bfa', '#fb7185', '#94a3b8', '#7dd3fc', '#86efac'];
const ACCENT_NAMES = ['amber', 'teal', 'violet', 'rose', 'slate', 'sky', 'mint'];

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return h;
}

function lastSegment(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? '';
}

// The project's folder name; a branch the machine made for the agent (`agent/...`) is internal, so it is not shown.
function workspaceName(v: View, wt: WorktreeV): string {
  const folder = lastSegment(v.checkouts.get(wt.checkoutId)?.path ?? '');
  const branch = wt.branch.startsWith('agent/') ? '' : wt.branch;
  if (folder) return branch ? `${folder} (${branch})` : folder;
  return branch || lastSegment(wt.path);
}

// What Home renders. A waiting approval makes its agent "needs you" even if the agent has not said so.
export function tiles(v: View): AgentTileModel[] {
  const waiting = new Set([...v.approvals.values()].map((a) => a.agentId));
  const sharingCounts = new Map<string, number>();
  for (const [id, agent] of v.agents) { const workspace = v.agentWorkspaces.get(id); if (agent.running && workspace) sharingCounts.set(workspace, (sharingCounts.get(workspace) ?? 0) + 1); }
  return [...v.agents.entries()].map(([id, a]) => {
    const i = hash(id) % ACCENTS.length;
    const node = v.nodes.get(a.nodeId);
    const wt = v.worktrees.get(a.worktreeId);
    const workspaceId = v.agentWorkspaces.get(id);
    const selected = workspaceId ? v.workspaces.get(workspaceId) : undefined;
    const sharing = !!workspaceId && (sharingCounts.get(workspaceId) ?? 0) > 1;
    const state = waiting.has(id) ? 'needs-you' : uiState(a.state);
    return {
      id,
      name: `${a.kind} · ${ACCENT_NAMES[i]}`,
      accent: ACCENTS[i] as string,
      machine: node?.hostname || a.nodeId,
      machineId: a.nodeId,
      workspace: selected ? selected.path : wt ? workspaceName(v, wt) : '',
      shared: sharing, git: selected ? !!selected.gitRoot : true, projectId: selected?.projectId ?? (wt ? v.checkouts.get(wt.checkoutId)?.projectId : undefined),
      state,
      preview: '',
      since: a.sinceMs || undefined,
      tokens: a.tokens,
      reportedBy: a.reported ? 'agent' : 'screen',
      workspaceRef: selected && workspaceId ? { node: a.nodeId, id: workspaceId.slice(workspaceId.indexOf('/') + 1), directory: true } : wt ? { node: a.nodeId, id: a.worktreeId.slice(a.worktreeId.indexOf('/') + 1) } : undefined,
      project: selected ? lastSegment(selected.gitRoot || selected.path) : wt ? lastSegment(v.checkouts.get(wt.checkoutId)?.path ?? '') || undefined : undefined,
      terminal: v.withTerminal.has(id),
      branch: selected?.branch || wt?.branch || undefined,
      filesChanged: selected?.filesChanged ?? wt?.filesChanged ?? 0,
    };
  });
}

export function machinesOnline(v: View): number {
  return [...v.nodes.values()].filter((n) => n.online).length;
}

export interface ApprovalModel {
  id: string; // hub-wide key `<node>/<local id>`
  node: string;
  localId: string;
  agentId: string;
  agentName: string;
  action: string;
  detail: string;
  expiresAtMs: number;
}

// What the approvals section shows: soonest to expire first.
export function approvalList(v: View): ApprovalModel[] {
  const names = new Map(tiles(v).map((t) => [t.id, t.name]));
  return [...v.approvals.entries()]
    .map(([id, a]) => {
      const at = id.indexOf('/');
      return {
        id,
        node: at < 0 ? '' : id.slice(0, at),
        localId: at < 0 ? id : id.slice(at + 1),
        agentId: a.agentId,
        agentName: names.get(a.agentId) ?? a.agentId,
        action: a.action,
        detail: a.detail,
        expiresAtMs: a.expiresAtMs,
      };
    })
    .sort((a, b) => a.expiresAtMs - b.expiresAtMs);
}

export interface MachineModel { id: string; name: string; os: string; online: boolean; agentsRunning: number; lastSeenMs: number; memoryBytes?: number }

// Online machines first, then by name.
export function machines(v: View): MachineModel[] {
  return [...v.nodes.entries()]
    .map(([id, n]) => ({ id, name: n.hostname || id, os: n.os, online: n.online, agentsRunning: n.agentsRunning, lastSeenMs: n.lastEventMs, memoryBytes: n.memoryBytes }))
    .sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name));
}

export interface ProjectChoice { node: string; projectId: string; label: string; path?: string }

// Where an agent can be started: every project checked out on a machine that is online.
export function projectChoices(v: View): ProjectChoice[] {
  const choices = new Map<string, ProjectChoice>();
  for (const c of [...v.checkouts.values(), ...v.workspaces.values()]) {
    if (v.nodes.get(c.nodeId)?.online) choices.set(`${c.nodeId}/${c.path}`, { node: c.nodeId, projectId: c.projectId, path: c.path,
      label: `${v.nodes.get(c.nodeId)?.hostname || c.nodeId} · ${lastSegment(c.path) || c.projectId}` });
  }
  return [...choices.values()].sort((a, b) => a.label.localeCompare(b.label));
}

export interface MessageModel { from: string; to: string; body: string; atMs: number; crossMachine: boolean }

// The newest messages first, with agent names instead of ids. A message between two machines says so.
export function recentMessages(v: View, limit = 5): MessageModel[] {
  const names = new Map(tiles(v).map((t) => [t.id, t.name]));
  const label = (id: string) => names.get(id) ?? id;
  const machine = (id: string) => id.split('/')[0];
  return v.messages.slice(-limit).reverse().map((m) => ({
    from: label(m.from), to: label(m.to), body: m.body, atMs: m.atMs,
    crossMachine: m.to.includes('/') && machine(m.from) !== machine(m.to),
  }));
}

export interface BoardTaskModel { id: string; title: string; status: 'open' | 'claimed' | 'done'; owner: string }
export interface BoardLeaseModel { path: string; holder: string; expiresAtMs: number }
export interface BoardFolderModel { nodeId: string; machine: string; path: string }
export interface BoardModel { projectId: string; project: string; folders: BoardFolderModel[]; tasks: BoardTaskModel[]; leases: BoardLeaseModel[] }

// The shared boards, one per project, with agent names instead of ids. A board only exists once something happened on it.
export function boards(v: View): BoardModel[] {
  const agents = new Map(tiles(v).map((t) => [t.id, t]));
  const label = (id: string) => agents.get(id)?.name ?? id;
  // Index once by logical identity; selected subdirectories in a Git checkout describe the same project root.
  const contexts = new Map<string, { project: string; folders: Map<string, BoardFolderModel> }>();
  const addFolder = (project: string, nodeId: string, path: string) => {
    if (!project || !nodeId || !path) return;
    let context = contexts.get(project);
    if (!context) contexts.set(project, (context = { project: lastSegment(path) || project, folders: new Map() }));
    context.folders.set(`${nodeId}\u0000${path}`, { nodeId, machine: v.nodes.get(nodeId)?.hostname || nodeId, path });
  };
  for (const c of v.checkouts.values()) addFolder(c.projectId, c.nodeId, c.path);
  for (const w of v.workspaces.values()) if (!w.managed) addFolder(w.projectId, w.nodeId, w.gitRoot || w.path);
  const by = new Map<string, BoardModel>();
  const board = (id: string) => {
    let b = by.get(id);
    if (!b) {
      const context = contexts.get(id);
      const folders = [...(context?.folders.values() ?? [])].sort((a, b) => a.nodeId.localeCompare(b.nodeId) || a.path.localeCompare(b.path));
      by.set(id, (b = { projectId: id, project: context?.project || id, folders, tasks: [], leases: [] }));
    }
    return b;
  };
  for (const [id, t] of v.tasks) {
    board(t.projectId).tasks.push({ id, title: t.title, status: t.done ? 'done' : t.ownerId ? 'claimed' : 'open', owner: t.ownerId ? label(t.ownerId) : '' });
  }
  // A lease belongs to its holder's project.
  for (const l of v.leases.values()) {
    const project = agents.get(l.agentId)?.projectId;
    if (!project) continue; // a holder this window does not know (yet) has no project to show it under
    board(project).leases.push({ path: l.path, holder: label(l.agentId), expiresAtMs: l.expiresAtMs });
  }
  return [...by.values()].sort((a, b) => a.project.localeCompare(b.project) || a.projectId.localeCompare(b.projectId));
}

export interface ProjectCost { label: string; microUsd: number }

// Estimated spend per project (the folder name of its checkout), biggest first. Only agents the hub had a price for
// count; `perAgent` is the hub's answer keyed by agent. Agents whose project is not known add to no row.
export function costByProject(v: View, perAgent: ReadonlyMap<string, number>): ProjectCost[] {
  const byProject = new Map<string, ProjectCost>();
  for (const [id, a] of v.agents) {
    const micro = perAgent.get(id);
    if (micro === undefined) continue;
    const checkout = v.checkouts.get(v.worktrees.get(a.worktreeId)?.checkoutId ?? '');
    if (!checkout) continue;
    const row = byProject.get(checkout.projectId) ?? { label: lastSegment(checkout.path) || checkout.projectId, microUsd: 0 };
    row.microUsd += micro;
    byProject.set(checkout.projectId, row);
  }
  return [...byProject.values()].sort((a, b) => b.microUsd - a.microUsd || a.label.localeCompare(b.label));
}

// "1.2 GB": what the agents on a machine hold in memory. Units are the same in every language.
export function bytesLabel(n: number): string {
  if (!(n > 0)) return '';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0, v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}
