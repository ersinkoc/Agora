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

export interface NodeV { hostname: string; os: string; online: boolean; lastEventMs: number; agentsRunning: number }
export interface WorktreeV { checkoutId: string; path: string; branch: string; filesChanged: number }
export interface AgentV {
  nodeId: string; worktreeId: string; kind: string; state: number; reported: boolean; running: boolean;
  sinceMs: number;
  tokens: number; // input + output so far
}
export interface CheckoutV { nodeId: string; projectId: string; path: string }
export interface ApprovalV { agentId: string; action: string; detail: string; expiresAtMs: number }

export interface MessageV { from: string; to: string; body: string; atMs: number }

export interface View {
  pos: number;
  messages: MessageV[]; // the latest agent-to-agent messages, oldest first
  nodes: Map<string, NodeV>;
  checkouts: Map<string, CheckoutV>;
  worktrees: Map<string, WorktreeV>;
  agents: Map<string, AgentV>;
  approvals: Map<string, ApprovalV>;
}

const MAX_MESSAGES = 50;

export function emptyView(): View {
  return { pos: 0, messages: [], nodes: new Map(), checkouts: new Map(), worktrees: new Map(), agents: new Map(), approvals: new Map() };
}

export function fromSnapshot(s: agora.v1.Snapshot.$Properties): View {
  const v = emptyView();
  v.pos = num(s.pos);
  for (const n of s.nodes ?? []) {
    v.nodes.set(n.nodeId ?? '', {
      hostname: n.hostname ?? '', os: n.os ?? '', online: !!n.online, lastEventMs: num(n.lastEventMs),
      agentsRunning: n.agentsRunning ?? 0,
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
  const at = num(ev.atMs);
  const node = v.nodes.get(producer);
  if (node) node.lastEventMs = Math.max(node.lastEventMs, at);
  const e = ev as Record<string, any>;
  switch (ev.body) {
    case 'nodeOnline': {
      const b = e.nodeOnline;
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
      if (n) n.agentsRunning = e.nodeResources.agentsRunning ?? 0;
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
    default:
      break; // terminals, usage, tasks and files do not change what the fleet screen shows
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
  return [...v.agents.entries()].map(([id, a]) => {
    const i = hash(id) % ACCENTS.length;
    const node = v.nodes.get(a.nodeId);
    const wt = v.worktrees.get(a.worktreeId);
    const state = waiting.has(id) ? 'needs-you' : uiState(a.state);
    return {
      id,
      name: `${a.kind} · ${ACCENT_NAMES[i]}`,
      accent: ACCENTS[i] as string,
      machine: node?.hostname || a.nodeId,
      workspace: wt ? workspaceName(v, wt) : '',
      state,
      preview: '',
      since: a.sinceMs || undefined,
      tokens: a.tokens,
      reportedBy: a.reported ? 'agent' : 'screen',
      workspaceRef: wt ? { node: a.nodeId, id: a.worktreeId.slice(a.worktreeId.indexOf('/') + 1) } : undefined,
      branch: wt?.branch || undefined,
      filesChanged: wt?.filesChanged ?? 0,
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
        agentName: names.get(a.agentId) ?? a.agentId,
        action: a.action,
        detail: a.detail,
        expiresAtMs: a.expiresAtMs,
      };
    })
    .sort((a, b) => a.expiresAtMs - b.expiresAtMs);
}

export interface MachineModel { id: string; name: string; os: string; online: boolean; agentsRunning: number; lastSeenMs: number }

// Online machines first, then by name.
export function machines(v: View): MachineModel[] {
  return [...v.nodes.entries()]
    .map(([id, n]) => ({ id, name: n.hostname || id, os: n.os, online: n.online, agentsRunning: n.agentsRunning, lastSeenMs: n.lastEventMs }))
    .sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name));
}

export interface ProjectChoice { node: string; projectId: string; label: string }

// Where an agent can be started: every project checked out on a machine that is online.
export function projectChoices(v: View): ProjectChoice[] {
  return [...v.checkouts.values()]
    .filter((c) => v.nodes.get(c.nodeId)?.online)
    .map((c) => ({ node: c.nodeId, projectId: c.projectId, label: `${v.nodes.get(c.nodeId)?.hostname || c.nodeId} · ${lastSegment(c.path) || c.projectId}` }))
    .sort((a, b) => a.label.localeCompare(b.label));
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
