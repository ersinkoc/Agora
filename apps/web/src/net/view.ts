// The client's copy of the hub's view (crates/agora-hub/src/projection.rs): built from a Snapshot, then advanced by
// positioned events. Keys are hub-wide `<producer>/<local id>`. Pure: no sockets, no React.

import { agora } from '../gen/agora.js';
import type { QuotaObs } from './quota';
import { SKIP_ACTION, skipOf, type SkipRequest } from './skipApproval';
import { AGENT_COLOUR_NAMES } from '../themes/tokens';
import { detailLaunchOptions, type StartOption } from './approval';
import { ALWAYS_LIST_MAX, alwaysOffer, type AlwaysOffer } from './always';
import { type AgentTileModel, type CommandModel, type ConfigCategory, type ConfigOptionModel, type SessionModel, type SessionOptionModel, type UiState, uiState } from '../model';
import { visible } from './hidden';
export { HIDDEN_RANGES, visible } from './hidden';

type Ev = agora.v1.Event.$Properties;
type Num = number | bigint | { toString(): string } | null | undefined;

// uint64 fields arrive as number, Long or bigint depending on the generator; ids and times stay below 2^53.
export function num(x: Num): number {
  if (x === null || x === undefined) return 0;
  return typeof x === 'number' ? x : Number(x.toString());
}

// `linkGen`: the generation of the latest `NodeLink { up }` this window saw (ADR-0070); unknown (0) for a node it only knows from the
// snapshot.
// `forgotten`: the owner had the hub forget it (ADR-0073); it and what it runs are not shown until it connects again.
export interface NodeV { hostname: string; os: string; online: boolean; lastEventMs: number; agentsRunning: number; memoryBytes?: number; build?: string; terminalsOpen?: number; linkGen?: number; forgotten?: boolean; binary?: string }
export interface WorkspaceV { nodeId: string; projectId: string; path: string; gitRoot: string; branch: string; managed: boolean; filesChanged: number }
export interface WorktreeV { checkoutId: string; path: string; branch: string; filesChanged: number }
export interface AgentV {
  nodeId: string; worktreeId: string; kind: string; state: number; reported: boolean; running: boolean;
  sinceMs: number;
  tokens: number; // input + output so far
  startedMs?: number; // when this window saw it start (not known for agents that were already running)
  exitCode?: number; // how it ended, once it has
  session?: SessionModel; // ADR-0058: options, context and its own cost, once reported
  parentId?: string; // ADR-0057: the hub-wide key of the agent that started it
  interrupted?: InterruptedV; // ADR-0060: it was running when its machine stopped; cleared by its next start
  skipsPermissions?: boolean; // ADR-0059: started with an option that skips permissions, for its whole life
  hidden?: boolean; // ADR-0073: still running when its machine was forgotten; shown again only if still running when it comes back
  // ADR-0101: the "always" answers its process holds (`AgentAlwaysAllowed`; the snapshot names actions only), newest last, at most 16.
  alwaysAllowed?: { id: string; action: string }[];
  // ADR-0094: the kind id of the agent a person runs by hand in this Shell (`AgentRunsInside`), while it runs.
  runsInside?: string;
  // ADR-0123: the launch settings it was started with (`AgentStarted.launch_settings`), and the model its usage last named (or, from a
  // snapshot, the model the hub worked out).
  launchSettings?: { id: string; value: string }[];
  usageModel?: string;
}

// ADR-0123: the model an agent runs on and its reasoning effort, one way for every kind (as the hub's snapshot works them out): the
// adapter's current model (its name when it lists it), else the launch setting `model`, else the model its usage named; the adapter's
// thought-level option (its choice's name), else the launch setting `effort`. Empty when not known. Every word is the agent's own.
export function modelAndEffort(a: Pick<AgentV, 'session' | 'launchSettings' | 'usageModel'>): { model: string; effort: string } {
  const setting = (id: string) => a.launchSettings?.find((s) => s.id === id)?.value ?? '';
  const s = a.session;
  const current = s?.currentModel ? (s.models.find((m) => m.id === s.currentModel)?.name || s.currentModel) : '';
  const level = s?.config?.find((o) => o.category === 'thought_level' && o.current);
  const effort = level ? (level.choices.find((c) => c.id === level.current)?.name || level.current) : '';
  return { model: current || setting('model') || a.usageModel || '', effort: effort || setting('effort') };
}
export interface InterruptedV { resumable: boolean; atMs: number; folder: string; stillRunning?: boolean }

type OptionWire = { id?: string | null; name?: string | null; description?: string | null; skipsPermissions?: boolean | null };
const optionsOf = (l: readonly OptionWire[] | null | undefined, modes: boolean): SessionOptionModel[] =>
  (l ?? []).map((o) => ({ id: o.id ?? '', name: o.name ?? '', description: o.description ?? '', skipsPermissions: modes && !!o.skipsPermissions })).filter((o) => o.id);
// ADR-0090: an agent's settings and slash commands, as the hub sent them (`SessionConfigCategory` 1..5, `SessionConfigType` 1 select, 2 boolean).
// Bounded here too (a node holds to 64 options of 64 choices and 100 commands); an option with no id cannot be set and is dropped.
const CATEGORIES: readonly ConfigCategory[] = ['', 'mode', 'model', 'model_config', 'thought_level', 'custom'];
type ConfigWire = { id?: string | null; name?: string | null; description?: string | null; category?: number | null; type?: number | null; current?: string | null; choices?: readonly OptionWire[] | null; skipsPermissions?: boolean | null };
export function configOf(list: readonly ConfigWire[] | null | undefined): ConfigOptionModel[] {
  return (list ?? []).filter((o) => o.id).slice(0, 64).map((o) => ({
    id: o.id ?? '', name: o.name ?? '', description: o.description ?? '', category: CATEGORIES[o.category ?? 0] ?? '',
    type: o.type === 2 ? 'boolean' as const : 'select' as const, current: o.current ?? '',
    choices: (o.choices ?? []).filter((c) => c.id).slice(0, 64).map((c) => ({ id: c.id ?? '', name: c.name ?? '', description: c.description ?? '', skipsPermissions: !!c.skipsPermissions })),
    skipsPermissions: !!o.skipsPermissions,
  }));
}
export function commandsOf(list: readonly { name?: string | null; description?: string | null; inputHint?: string | null }[] | null | undefined): CommandModel[] {
  const seen = new Set<string>();
  // The first of a repeated name wins (the hub dedupes too): two rows with one name would act as one.
  return (list ?? []).filter((c) => { const n = (c.name ?? '').replace(/^\/+/, '').toLowerCase(); if (!n || seen.has(n)) return false; seen.add(n); return true; }).slice(0, 100).map((c) => ({ name: c.name ?? '', description: c.description ?? '', inputHint: c.inputHint ?? '' }));
}
const emptySession = (): SessionModel => ({ modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 0, contextWindow: 0 });

// ADR-0058: a usage report's context level and the agent's own cost, folded like the hub does (a new currency starts the sum again).
function addUsage(a: AgentV, u: Record<string, any>): void {
  const ctx = num(u.contextTokens), size = num(u.contextWindow);
  const hasCost = u.costMicros !== undefined && u.costMicros !== null && !!u.currency;
  if (!ctx && !size && !hasCost) return;
  const s = a.session ?? emptySession();
  if (ctx || size) { s.contextTokens = ctx; s.contextWindow = size; }
  if (hasCost) {
    if (s.currency !== u.currency) { s.currency = u.currency; s.costMicros = 0; }
    s.costMicros = (s.costMicros ?? 0) + num(u.costMicros);
  }
  a.session = s;
}
export interface CheckoutV { nodeId: string; projectId: string; path: string }
// ADR-0101: `scopes`, `alwaysLabel` and `alwaysOptionId` as the machine announced them (after its policy).
export interface ApprovalV { agentId: string; action: string; detail: string; expiresAtMs: number; scopes?: number[]; alwaysLabel?: string; alwaysOptionId?: string }

// ADR-0101: the always fields of a request as they came (nothing trimmed: an always answer is signed over them exactly).
const alwaysFields = (p: { scopes?: readonly (number | null)[] | null; alwaysLabel?: string | null; alwaysOptionId?: string | null }): Pick<ApprovalV, 'scopes' | 'alwaysLabel' | 'alwaysOptionId'> =>
  p.scopes?.length ? { scopes: p.scopes.map((x) => Number(x ?? 0)), alwaysLabel: p.alwaysLabel ?? '', alwaysOptionId: p.alwaysOptionId ?? '' } : {};

const HANDOFF_STATES = ['pending', 'delivered', 'failed', 'unresolved'] as const;
type HandoffState = typeof HANDOFF_STATES[number];

export interface HandoffDeliveryModel {
  id: string;
  receiver: string;
  state: HandoffState;
  errorCode: string;
  acknowledged: boolean;
}
interface HandoffDeliveryV extends HandoffDeliveryModel { taskKey: string }

const HANDOFF_SETTLED_MAX = 256;
export const handoffSettled = (delivery: HandoffDeliveryModel) =>
  delivery.state === 'delivered' || delivery.state === 'failed' || delivery.acknowledged;

function recordHandoff(v: View, row: agora.v1.TaskHandoffDeliveryChanged.$Properties): void {
  const id = row.deliveryId ?? '';
  const state = HANDOFF_STATES[Number(row.state ?? 0) - 1];
  if (!id || !state || !row.projectId || !row.taskId) return;
  const delivery = {
    id, state, taskKey: `${row.projectId}/${row.taskId}`, receiver: row.receiverAgentId ?? '',
    errorCode: row.errorCode ?? '', acknowledged: row.acknowledged ?? false,
  };
  v.handoffDeliveries.delete(id);
  v.handoffDeliveries.set(id, delivery);
  if (handoffSettled(delivery)) {
    const settled = [...v.handoffDeliveries.values()].filter(handoffSettled);
    if (settled.length > HANDOFF_SETTLED_MAX) v.handoffDeliveries.delete(settled[0]!.id);
  }
}

export interface TaskReviewModel {
  revision: number;
  state: 'pending' | 'accepted' | 'returned';
  decidedAtMs?: number;
}

function recordReview(v: View, key: string, row: agora.v1.TaskReview.$Properties): void {
  const revision = num(row.reportRevision);
  const states = agora.v1.TaskReviewState;
  const state = row.state === states.TASK_REVIEW_STATE_PENDING ? 'pending'
    : row.state === states.TASK_REVIEW_STATE_ACCEPTED ? 'accepted'
    : row.state === states.TASK_REVIEW_STATE_RETURNED ? 'returned' : undefined;
  if (revision > 0 && state) {
    const decidedAtMs = num(row.decidedAtMs);
    v.reviews.set(key, { revision, state, ...(decidedAtMs > 0 && decidedAtMs <= 8.64e15 ? { decidedAtMs } : {}) });
  }
}

export interface TaskV { projectId: string; title: string; ownerId: string; done: boolean; parentTaskId?: string }
export interface LeaseV { agentId: string; path: string; expiresAtMs: number }
// `producer` is the machine that recorded it; `paired` marks a copy already matched with its twin (#905). `via` is the machine whose
// word alone names the sender: a copy recorded by another machine than the sender's (a receiver's, or a forged one) that neither the
// sender's machine nor the hub has confirmed.
export interface MessageV { from: string; to: string; body: string; atMs: number; producer?: string; paired?: boolean; via?: string }

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
  handoffDeliveries: Map<string, HandoffDeliveryV>;
  reviews: Map<string, TaskReviewModel>;
  reviewProjects: Set<string>;
  tasks: Map<string, TaskV>; // the shared board's tasks seen since this window opened (ADR-0039)
  leases: Map<string, LeaseV>;
  withTerminal: Set<string>; // agents that have a terminal; the others (chat agents) are talked to with prompts
  // ADR-0053: agents forgotten while this window watched. The hub keeps them, hidden, so a later `AgentInterrupted` about one does not
  // list it again; only a new `AgentStarted` of the id does.
  forgottenAgents?: Set<string>;
  // ADR-0061: node id -> the devices it says it trusts (its last `NodeTrustedDevices`).
  trusted?: Map<string, string[]>;
  // ADR-0091: the newest plan-limit observation per machine, tool and window (node, then `<tool>\0<window>`); at most `MAX_QUOTA` windows per machine.
  quota?: Map<string, Map<string, QuotaV>>;
  // #1150: machine -> the ids of its last `RESOLVED_KEPT` decided approvals seen since the snapshot, oldest first (a Set keeps insertion
  // order). A request a machine records after its decision brings no card back, as in the hub's view (`View::resolved_approvals`).
  resolved?: Map<string, Set<string>>;
}
export interface QuotaV { nodeId: string; tool: string; window: string; usedPercent: number; resetsAtMs: number; observedAtMs: number }
const MAX_QUOTA = 64; // per machine, as the hub
const QUOTA_SKEW_MS = 120_000;

const MAX_MESSAGES = 50;
const RESOLVED_KEPT = 256; // decided approvals remembered per machine, as the hub (`RESOLVED_APPROVALS_KEPT`)

export function emptyView(): View {
  return { pos: 0, workspaces: new Map(), agentWorkspaces: new Map(), messages: [], nodes: new Map(), checkouts: new Map(), worktrees: new Map(), agents: new Map(), approvals: new Map(), tasks: new Map(), reviews: new Map(), reviewProjects: new Set(), handoffDeliveries: new Map(), leases: new Map(), withTerminal: new Set() };
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
      memoryBytes: Number(n.memoryUsedBytes ?? 0), build: n.build ?? '', terminalsOpen: n.terminalsOpen ?? 0,
      // ADR-0069: the node's program, only when its folder is not on that machine's PATH.
      binary: n.binary ?? '',
    });
  }
  for (const n of s.nodes ?? []) if (n.trustedDeviceIds?.length) (v.trusted ??= new Map()).set(n.nodeId ?? '', [...n.trustedDeviceIds]);
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
      parentId: a.parentAgentId || remoteParent(a.parentNodeId, a.parentNodeAgentId),
      ...(a.interrupted ? { interrupted: { resumable: !!a.resumable, atMs: num(a.interruptedAtMs), folder: a.folder ?? '', stillRunning: !!a.stillRunning }, sinceMs: num(a.interruptedAtMs) } : {}),
      skipsPermissions: !!a.skipsPermissions,
      // #902: how a closed agent ended (`AgentSnapshot.exit_code`), so a reloaded window still shows one that failed as failed.
      ...(!a.running && a.exitCode !== undefined && a.exitCode !== null ? { exitCode: a.exitCode } : {}),
      ...(a.running && a.alwaysAllowed?.length ? { alwaysAllowed: a.alwaysAllowed.slice(-ALWAYS_LIST_MAX).map((action) => ({ id: '', action })) } : {}),
      ...(a.running && a.runsInside && insideKind(a.runsInside) ? { runsInside: a.runsInside } : {}),
      ...(a.launchSettings?.length ? { launchSettings: a.launchSettings.slice(0, 8).map((s) => ({ id: s.id ?? '', value: s.value ?? '' })) } : {}),
      ...(a.model ? { usageModel: a.model } : {}),
    });
    const modes = optionsOf(a.modes, true), models = optionsOf(a.models, false);
    const reported = a.costMicros !== undefined && a.costMicros !== null && !!a.currency;
    if (modes.length || models.length || num(a.contextTokens) || num(a.contextWindow) || reported) {
      const agent = v.agents.get(a.agentId ?? '');
      if (agent) agent.session = {
        modes, models, currentMode: a.currentMode ?? '', currentModel: a.currentModel ?? '',
        contextTokens: num(a.contextTokens), contextWindow: num(a.contextWindow),
        ...(reported ? { costMicros: num(a.costMicros), currency: a.currency ?? '' } : {}),
      };
    }
  }
  for (const tm of s.terminals ?? []) if (tm.agentId) v.withTerminal.add(tm.agentId);
  for (const project of s.reviewEnabledProjectIds ?? []) v.reviewProjects.add(project);
  for (const t of s.tasks ?? []) {
    const key = `${t.projectId ?? ''}/${t.taskId ?? ''}`;
    v.tasks.set(key, { projectId: t.projectId ?? '', title: t.title ?? '', ownerId: t.claimedByAgentId ?? '', done: !!t.done, ...(t.parentTaskId ? { parentTaskId: t.parentTaskId } : {}) });
    if (t.review) recordReview(v, key, t.review);
  }
  for (const l of s.leases ?? []) {
    v.leases.set(`${l.agentId}\u0000${l.path}`, { agentId: l.agentId ?? '', path: l.path ?? '', expiresAtMs: num(l.expiresAtMs) });
  }
  for (const p of s.approvals ?? []) {
    v.approvals.set(p.approvalId ?? '', {
      agentId: p.agentId ?? '', action: p.action ?? '', detail: p.detail ?? '', expiresAtMs: num(p.expiresAtMs), ...alwaysFields(p),
    });
  }
  for (const row of s.taskHandoffDeliveries ?? []) recordHandoff(v, row);
  foldRecentMessages(v, s.recentMessages ?? []);
  return v;
}

// One mesh message, live or from the snapshot (ADR-0103), recorded by `producer` at `at`.
function postMessage(v: View, producer: string, at: number, b: agora.v1.MessagePosted.$Properties): void {
  const q = (id: string) => `${producer}/${id}`;
  const to = String(b.to ?? '');
  const from = String(b.fromAgentId ?? '');
  // #905: a message between machines is recorded by both: the sender's node (`to` is `<node>/<agent>`) and the receiver's,
  // whose `from` is already the hub-wide `<origin>/<agent>` (ADR-0031), so it is not qualified again. Message ids are per node
  // (ADR-0074), so the two copies are matched by sender, recipient and text from two machines, one with one: a message sent
  // twice stays two rows. A node may write any `from`: one that names another machine's agent is shown with the machine that
  // said so until the sender's machine or the hub (the shared board, which writes hub-wide ids) records the same message.
  const qualified = from.includes('/');
  const sender = qualified ? from : q(from);
  const vouched = !qualified || producer === 'hub' || producer === sender.split('/')[0];
  const m: MessageV = { from: sender, to: to.includes('/') || to.startsWith('#') ? to : q(to), body: b.body ?? '', atMs: at, producer, ...(vouched ? {} : { via: producer }) };
  // Two copies nobody confirmed (a receiver's and one some other node wrote) never vouch for each other.
  const twin = v.messages.find((x) => !x.paired && x.producer !== producer && (vouched || !x.via) && x.from === m.from && x.to === m.to && x.body === m.body);
  if (twin) {
    twin.paired = true;
    if (vouched) delete twin.via;
    return;
  }
  v.messages.push(m);
  if (v.messages.length > MAX_MESSAGES) v.messages.shift();
}

// ADR-0103: the snapshot's newest messages, oldest first, folded in as the live ones are (pairing, `via`, the cap), but not through
// `applyEvent`'s position rule: their positions are at or before the snapshot's, which it would drop. Each position is folded once;
// one after the snapshot's is left to the live stream, which brings it. Any other body is ignored: history carries messages only.
function foldRecentMessages(v: View, recent: readonly agora.v1.PositionedEvent.$Properties[]): void {
  const folded = new Set<number>();
  const entries = recent
    .map((pe) => ({ pos: num(pe.pos), ev: pe.event }))
    .filter((x) => x.pos <= v.pos)
    .sort((a, b) => a.pos - b.pos);
  for (const { pos, ev } of entries) {
    const b = ev?.messagePosted;
    if (!ev || !b || folded.has(pos)) continue;
    folded.add(pos);
    postMessage(v, ev.producerId ?? '', num(ev.atMs), b);
  }
}

// Applies one event at log position `pos`. An event at or before the view's position was already folded in
// (a resume replays the tail), so it is ignored: applying is idempotent.
export function applyEvent(v: View, pos: number, ev: Ev): void {
  // The hub sends session options and board metadata that did not fit at the snapshot's position.
  const hubMetadata = ev.producerId === 'hub' && (
    ev.body === 'taskHandoffDeliveryChanged' || ev.body === 'taskReviewChanged' || ev.body === 'taskReviewEnabled' || ev.body === 'taskParentLinked'
  );
  const replacing = pos === v.pos && (hubMetadata || ev.body === 'agentSessionConfig' || ev.body === 'agentCommands' || ev.body === 'agentQuotaObserved');
  if (pos <= v.pos && !replacing) return;
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
    // A folder is registered again for each agent started in it, with no count (#1017): the machine's Git watcher counts it and says
    // it again only when it changes, so a count already known for the folder stays.
    case 'workspaceRegistered': { const w = e.workspaceRegistered; const known = v.workspaces.get(q(w.workspaceId)); v.workspaces.set(q(w.workspaceId), { nodeId: producer, projectId: w.projectId ?? '', path: w.path ?? '', gitRoot: w.gitRoot ?? '', branch: w.branch ?? '', managed: !!w.managedWorktree, filesChanged: w.filesChanged || known?.filesChanged || 0 }); break; }
    case 'agentWorkspaceBound': v.agentWorkspaces.set(q(e.agentWorkspaceBound.agentId), q(e.agentWorkspaceBound.workspaceId)); break;
    case 'workspaceStatusChanged': { const w = v.workspaces.get(q(e.workspaceStatusChanged.workspaceId)); if (w) w.filesChanged = e.workspaceStatusChanged.filesChanged ?? 0; break; }
    case 'nodeOnline': {
      const b = e.nodeOnline;
      // The node's process started: what it ran before is gone, and its waiting approvals are lost.
      // F1421: an ended agent can be asked nothing; its settings and commands go, as after `agentExited` (a later snapshot has none).
      for (const a of v.agents.values()) if (a.nodeId === b.nodeId && a.running) { a.running = false; a.state = 4 /* done */; a.alwaysAllowed = undefined; a.runsInside = undefined; if (a.session) a.session = { ...a.session, config: undefined, commands: undefined }; }
      for (const k of [...v.approvals.keys()]) if (k.startsWith(`${b.nodeId}/`)) v.approvals.delete(k);
      for (const k of [...v.withTerminal]) if (k.startsWith(`${b.nodeId}/`)) v.withTerminal.delete(k);
      const was = v.nodes.get(b.nodeId);
      // The start says which program it is (`build`) and, ADR-0069, where it runs from when that folder is not on the PATH. The last
      // resources it reported stay until it reports new ones, as on the hub.
      v.nodes.set(b.nodeId, { hostname: b.hostname ?? '', os: b.os ?? '', online: true, lastEventMs: Math.max(at, was?.lastEventMs ?? 0), agentsRunning: was?.agentsRunning ?? 0, memoryBytes: was?.memoryBytes ?? 0, terminalsOpen: was?.terminalsOpen ?? 0, linkGen: was?.linkGen, build: b.build ?? '', binary: b.binary ?? '' });
      break;
    }
    // ADR-0070: the hub's record of a node's connection decides whether the machine is online; it never touches what the machine runs
    // (a link that goes down and up is not a process start). Only the hub writes it. A fact about a connection older than the latest
    // one that opened, or about a replaced one, changes nothing but "last seen".
    case 'nodeLink': {
      if (producer !== 'hub') break;
      const b = e.nodeLink;
      const id = b.nodeId ?? '';
      let n = v.nodes.get(id);
      if (!n) {
        if (!b.up) break;
        n = { hostname: '', os: '', online: true, lastEventMs: 0, agentsRunning: 0 };
        v.nodes.set(id, n);
      }
      n.lastEventMs = Math.max(n.lastEventMs, num(b.lastHeardMs));
      const gen = num(b.generation);
      const stale = gen < (n.linkGen ?? 0) || b.reason === agora.v1.NodeLinkReason.NODE_LINK_REASON_REPLACED;
      if (!stale) {
        n.online = !!b.up;
        if (b.up) n.linkGen = gen;
        // ADR-0073: a forgotten machine that connects again is shown again, with what it still runs; one that stopped while it was
        // hidden stays forgotten, as the hub's `remember_node` keeps it.
        if (b.up && n.forgotten) {
          n.forgotten = false;
          for (const [key, a] of [...v.agents]) {
            if (a.nodeId !== id || !a.hidden) continue;
            if (a.running) a.hidden = false;
            else { v.agents.delete(key); v.agentWorkspaces.delete(key); v.withTerminal.delete(key); (v.forgottenAgents ??= new Set()).add(key); }
          }
        }
      }
      break;
    }
    // ADR-0073: only the hub writes it, for an offline machine. Its closed agents stay forgotten for good; what still runs there is
    // kept, hidden with the machine, and shown again if it comes back.
    case 'nodeForgotten': {
      if (producer !== 'hub') break;
      const id = e.nodeForgotten.nodeId ?? '';
      const n = v.nodes.get(id);
      if (n) n.forgotten = true;
      v.resolved?.delete(id); // #1150: its decisions are not remembered for it, as in the hub's view
      for (const [key, a] of [...v.agents]) {
        if (a.nodeId !== id) continue;
        if (a.running) a.hidden = true;
        else { v.agents.delete(key); v.agentWorkspaces.delete(key); v.withTerminal.delete(key); (v.forgottenAgents ??= new Set()).add(key); }
      }
      break;
    }
    case 'nodeTrustedDevices': (v.trusted ??= new Map()).set(producer, [...(e.nodeTrustedDevices.deviceIds ?? [])]); break;
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
      if (n) { n.agentsRunning = e.nodeResources.agentsRunning ?? 0; n.memoryBytes = Number(e.nodeResources.memoryUsedBytes ?? 0); n.terminalsOpen = e.nodeResources.terminalsOpen ?? n.terminalsOpen; }
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
      v.forgottenAgents?.delete(q(b.agentId));
      // ADR-0104 (#930): the node re-states a process it announced before under this id (taken up from the holder after its restart,
      // or in a resync). Its tokens, context level, own cost and start time stay, for that machine's own agent only; a new start, a
      // resume, a restart after a crash and an older node's start begin again from 0. The rest starts again as for any start (the
      // node re-states modes, settings, commands and always answers).
      const was = b.sameProcess ? v.agents.get(q(b.agentId)) : undefined;
      const kept = was && was.nodeId === producer ? was : undefined;
      const s = kept?.session;
      const session = s && (s.contextTokens || s.contextWindow || s.currency)
        ? { ...emptySession(), contextTokens: s.contextTokens, contextWindow: s.contextWindow, ...(s.currency ? { costMicros: s.costMicros, currency: s.currency } : {}) }
        : undefined;
      v.agents.set(q(b.agentId), {
        nodeId: b.nodeId ?? '', worktreeId: q(b.worktreeId), kind: b.kind ?? '', state: 3 /* idle */,
        reported: false, running: true, sinceMs: at, tokens: kept ? kept.tokens : 0, startedMs: kept ? kept.startedMs : at,
        parentId: b.parentAgentId ? q(b.parentAgentId) : remoteParent(b.parentNodeId, b.parentNodeAgentId),
        skipsPermissions: !!b.skipsPermissions,
        ...(session ? { session } : {}),
        ...(b.launchSettings?.length ? { launchSettings: b.launchSettings.slice(0, 8).map((s: { id?: string | null; value?: string | null }) => ({ id: s.id ?? '', value: s.value ?? '' })) } : {}),
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
      if (a) {
        a.tokens += num(e.agentUsageReported.inputTokens) + num(e.agentUsageReported.outputTokens); addUsage(a, e.agentUsageReported);
        if (e.agentUsageReported.model) a.usageModel = e.agentUsageReported.model;
      }
      break;
    }
    case 'agentSessionOptions': {
      const b = e.agentSessionOptions;
      const a = v.agents.get(q(b.agentId));
      if (a) {
        const s = a.session ?? emptySession();
        a.session = { ...s, modes: optionsOf(b.modes, true), models: optionsOf(b.models, false), currentMode: b.currentMode ?? '', currentModel: b.currentModel ?? '' };
      }
      break;
    }
    case 'agentSessionConfig': {
      const a = v.agents.get(q(e.agentSessionConfig.agentId));
      if (a) a.session = { ...(a.session ?? emptySession()), config: configOf(e.agentSessionConfig.options) };
      break;
    }
    // ADR-0091: a machine speaks for itself only; the newest observation of a window wins; the table is bounded (the oldest goes).
    case 'agentQuotaObserved': {
      const b = e.agentQuotaObserved;
      if ((b.nodeId ?? '') !== producer || !b.tool || !b.window) break;
      const key = `${b.tool}\u0000${b.window}`;
      const table = ((v.quota ??= new Map()).get(producer) ?? new Map<string, QuotaV>());
      v.quota.set(producer, table);
      // Never newer than this window's own clock (plus skew): a machine whose clock runs ahead cannot win a window for good. The
      // comparison with the stored reading is made on the cut time.
      const observedAtMs = Math.min(num(b.observedAtMs), Date.now() + QUOTA_SKEW_MS);
      const old = table.get(key);
      if (old && old.observedAtMs > observedAtMs) break;
      table.set(key, { nodeId: producer, tool: b.tool, window: b.window, usedPercent: Math.min(100, num(b.usedPercent)), resetsAtMs: num(b.resetsAtMs), observedAtMs });
      if (table.size > MAX_QUOTA) {
        let oldest: string | undefined;
        for (const [k, x] of table) if (oldest === undefined || x.observedAtMs < (table.get(oldest)?.observedAtMs ?? 0)) oldest = k;
        if (oldest !== undefined) table.delete(oldest);
      }
      break;
    }
    case 'agentCommands': {
      const a = v.agents.get(q(e.agentCommands.agentId));
      if (a) a.session = { ...(a.session ?? emptySession()), commands: commandsOf(e.agentCommands.commands) };
      break;
    }
    // ADR-0053: the hub stopped listing a closed agent (its id is already the hub-wide key).
    case 'agentForgotten': {
      const id = e.agentForgotten.agentId ?? '';
      v.agents.delete(id);
      v.agentWorkspaces.delete(id);
      v.withTerminal.delete(id);
      (v.forgottenAgents ??= new Set()).add(id);
      break;
    }
    // ADR-0060: kept with the closed agents, offering Resume (its own session) or Start again; one never seen is listed from the event.
    case 'agentInterrupted': {
      const b = e.agentInterrupted;
      const key = q(b.agentId);
      if (!v.agents.has(key) && v.forgottenAgents?.has(key)) break; // the hub keeps it forgotten (ADR-0053)
      const a: AgentV = v.agents.get(key) ?? {
        nodeId: producer, worktreeId: q(''), kind: b.kind ?? '', state: 4, reported: false, running: false, sinceMs: 0, tokens: 0,
        parentId: b.parentAgentId ? q(b.parentAgentId) : remoteParent(b.parentNodeId, b.parentNodeAgentId),
      };
      a.running = false;
      a.state = 4 /* done */;
      a.alwaysAllowed = undefined; // ADR-0101: the hub clears it too
      a.runsInside = undefined; // ADR-0094: the same
      a.sinceMs = num(b.interruptedAtMs) || at;
      a.interrupted = { resumable: !!b.resumable, atMs: num(b.interruptedAtMs), folder: b.folder ?? '', stillRunning: !!b.stillRunning };
      if (a.session) a.session = { ...a.session, config: undefined, commands: undefined }; // F1421: ended, as the hub drops them
      v.agents.set(key, a);
      dropSkipCards(v, key);
      break;
    }
    case 'agentExited': {
      dropSkipCards(v, q(e.agentExited.agentId));
      const a = v.agents.get(q(e.agentExited.agentId));
      if (a) {
        a.running = false; a.state = 4 /* done */; a.sinceMs = at; a.exitCode = e.agentExited.exitCode ?? 0;
        // ADR-0101: an always answer lives in the agent's process.
        a.alwaysAllowed = undefined;
        a.runsInside = undefined; // ADR-0094: and so does the program run inside it
        // ADR-0090: an ended agent can be asked nothing; its settings and commands are gone (the hub drops them too).
        if (a.session) a.session = { ...a.session, config: undefined, commands: undefined };
      }
      break;
    }
    // ADR-0101: an agent that runs holds one more "always" answer (one it holds already, said again after a restart, changes nothing).
    case 'agentAlwaysAllowed': {
      const b = e.agentAlwaysAllowed;
      const a = v.agents.get(q(b.agentId));
      if (!a?.running) break;
      const held = a.alwaysAllowed ?? [];
      const id = b.approvalId ?? '';
      if (id && held.some((h) => h.id === id)) break;
      a.alwaysAllowed = [...held, { id, action: b.action ?? '' }].slice(-ALWAYS_LIST_MAX);
      break;
    }
    // ADR-0094: what a person runs by hand in a running Shell, as its machine found it (empty: it ended). Anything but a plain kind id is
    // left out, as the hub leaves it.
    case 'agentRunsInside': {
      const b = e.agentRunsInside;
      const a = v.agents.get(q(b.agentId));
      if (!a?.running) break;
      const kind = b.kind ?? '';
      if (!kind) a.runsInside = undefined;
      else if (insideKind(kind)) a.runsInside = kind;
      break;
    }
    case 'terminalOpened':
      if (e.terminalOpened.agentId) v.withTerminal.add(q(e.terminalOpened.agentId));
      break;
    case 'approvalRequested': {
      const b = e.approvalRequested;
      if (v.resolved?.get(producer)?.has(b.approvalId ?? '')) break;
      v.approvals.set(q(b.approvalId), { agentId: q(b.agentId), action: b.action ?? '', detail: b.detail ?? '', expiresAtMs: num(b.expiresAtMs), ...alwaysFields(b) });
      break;
    }
    case 'approvalResolved': {
      const id = e.approvalResolved.approvalId ?? '';
      v.approvals.delete(q(id));
      v.resolved ??= new Map();
      const ids = v.resolved.get(producer) ?? new Set<string>();
      // A decision said again (an announcement restates recent ones) keeps its place.
      if (!ids.has(id)) {
        ids.add(id);
        if (ids.size > RESOLVED_KEPT) ids.delete(ids.values().next().value as string);
      }
      v.resolved.set(producer, ids);
      break;
    }
    case 'messagePosted':
      postMessage(v, producer, at, e.messagePosted);
      break;
    // The hub writes board events with ids that are already `<node>/<agent>`.
    case 'taskHandoffDeliveryChanged':
      if (producer === 'hub') recordHandoff(v, e.taskHandoffDeliveryChanged);
      break;
    case 'taskReviewEnabled':
      if (producer === 'hub' && e.taskReviewEnabled.projectId) v.reviewProjects.add(e.taskReviewEnabled.projectId);
      break;
    case 'taskReviewChanged': {
      const b = e.taskReviewChanged;
      if (producer === 'hub' && b.review) recordReview(v, `${b.projectId ?? ''}/${b.taskId ?? ''}`, b.review);
      break;
    }
    case 'taskParentLinked': {
      const b = e.taskParentLinked;
      if (producer !== 'hub' || !b.projectId || !b.taskId || !b.parentTaskId || b.taskId === b.parentTaskId) break;
      const key = `${b.projectId}/${b.taskId}`;
      const task = v.tasks.get(key);
      if (task) v.tasks.set(key, { ...task, parentTaskId: b.parentTaskId });
      break;
    }
    case 'taskChanged': {
      const b = e.taskChanged;
      const key = `${b.projectId ?? ''}/${b.taskId ?? ''}`;
      const parentTaskId = v.tasks.get(key)?.parentTaskId;
      if (producer !== 'hub' && (parentTaskId || v.reviews.has(key))) break;
      v.tasks.set(key, { projectId: b.projectId ?? '', title: b.title ?? '', ownerId: b.claimedByAgentId ? qb(b.claimedByAgentId) : '', done: !!b.done, ...(parentTaskId ? { parentTaskId } : {}) });
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

// An agent's colour is a variable per name (design/tokens.css `--agent-*`), so each theme draws it in a shade that shows on it.
const ACCENT_NAMES = AGENT_COLOUR_NAMES;
const ACCENTS = ACCENT_NAMES.map((n) => `var(--agent-${n})`);

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
/** ADR-0094: a kind id the window keeps from `AgentRunsInside` (the hub's rule): 1 to 64 of a-z, 0-9, `-`, `_`, `.`. */
export const insideKind = (kind: string): boolean => /^[a-z0-9._-]{1,64}$/.test(kind);

/** ADR-0094: the name a kind is shown by when nothing better is known: its id's words, each with a capital ("claude-code" → "Claude Code"). */
export const kindWords = (kind: string): string => kind.split(/[-_.]+/).filter(Boolean).map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(' ');

/** `kindName`: the display name a machine reported for one of its kinds (Settings → Agents keeps the last list), when this window has it. */
/** ADR-0123: an agent's provider: its kind's, as its machine's manifest names it, else the first part of a `provider/model` model id
 *  (a kind that uses many providers, such as opencode); empty when neither says. */
export function providerOf(kindProvider: string | undefined, model: string): string {
  if (kindProvider && /^[a-z0-9-]{1,32}$/.test(kindProvider)) return kindProvider;
  const at = model.indexOf('/');
  const first = at > 0 ? model.slice(0, at).toLowerCase() : '';
  return /^[a-z0-9-]{1,32}$/.test(first) ? first : '';
}

export function tiles(v: View, kindName?: (node: string, kind: string) => string | undefined, kindProvider?: (node: string, kind: string) => string | undefined): AgentTileModel[] {
  const waiting = new Set([...v.approvals.values()].map((a) => a.agentId));
  const sharingCounts = new Map<string, number>();
  for (const [id, agent] of v.agents) { const workspace = v.agentWorkspaces.get(id); if (agent.running && workspace) sharingCounts.set(workspace, (sharingCounts.get(workspace) ?? 0) + 1); }
  const list = [...v.agents.entries()].map(([id, a]) => {
    const i = hash(id) % ACCENTS.length;
    const node = v.nodes.get(a.nodeId);
    const wt = v.worktrees.get(a.worktreeId);
    const workspaceId = v.agentWorkspaces.get(id);
    const selected = workspaceId ? v.workspaces.get(workspaceId) : undefined;
    const sharing = !!workspaceId && (sharingCounts.get(workspaceId) ?? 0) > 1;
    const workspace = selected ? selected.path : wt ? workspaceName(v, wt) : a.interrupted?.folder ?? '';
    const project = selected ? lastSegment(selected.gitRoot || selected.path) : wt ? lastSegment(v.checkouts.get(wt.checkoutId)?.path ?? '') : '';
    const branch = selected?.branch || wt?.branch || '';
    // A closed agent is finished, whatever state it had when its machine lost it (a blocked one that ended is not waiting for anyone).
    const state = !a.running ? 'finished' : waiting.has(id) ? 'needs-you' : uiState(a.state);
    // ADR-0094: while a person runs an agent by hand in this Shell, the Shell is shown as that agent (its name and avatar), in its colour.
    const inside = a.running ? a.runsInside : undefined;
    const shownKind = inside ?? a.kind;
    const me = modelAndEffort(a);
    return {
      id,
      name: `${visible(shownKind)} · ${ACCENT_NAMES[i]}`,
      ...(shownKind ? { kind: shownKind } : {}),
      ...(inside && a.kind ? { hostKind: a.kind } : {}),
      accent: ACCENTS[i] as string,
      machine: visible(node?.hostname || a.nodeId),
      machineId: a.nodeId,
      workspace: visible(workspace),
      shared: sharing, git: selected ? !!selected.gitRoot : true, projectId: selected?.projectId ?? (wt ? v.checkouts.get(wt.checkoutId)?.projectId : undefined),
      state,
      preview: '',
      since: a.sinceMs || undefined,
      tokens: a.tokens,
      reportedBy: a.reported ? 'agent' : 'screen',
      workspaceRef: selected && workspaceId ? { node: a.nodeId, id: workspaceId.slice(workspaceId.indexOf('/') + 1), directory: true } : wt ? { node: a.nodeId, id: a.worktreeId.slice(a.worktreeId.indexOf('/') + 1) } : undefined,
      project: project ? visible(project) : undefined,
      terminal: v.withTerminal.has(id),
      branch: branch ? visible(branch) : undefined,
      filesChanged: selected?.filesChanged ?? wt?.filesChanged ?? 0,
      closed: !a.running,
      exitCode: a.exitCode,
      startedMs: a.startedMs,
      endedMs: a.running ? undefined : a.sinceMs || undefined,
      session: a.session,
      parentId: a.parentId,
      interrupted: a.interrupted ? { resumable: a.interrupted.resumable, stillRunning: !!a.interrupted.stillRunning } : undefined,
      skipsPermissions: !!a.skipsPermissions,
      ...me,
      provider: providerOf(shownKind ? kindProvider?.(a.nodeId, shownKind) : undefined, me.model),
      ...(a.running && a.alwaysAllowed?.length ? { alwaysAllowed: a.alwaysAllowed.map((h) => h.action) } : {}),
      ...(a.running && a.runsInside ? { runsInside: { kind: a.runsInside, name: kindName?.(a.nodeId, a.runsInside) || kindWords(a.runsInside) } } : {}),
    } as AgentTileModel;
  });
  const names = new Map(list.map((t) => [t.id, t.name]));
  for (const t of list) if (t.parentId) t.parentName = names.get(t.parentId) ?? t.parentId.slice(t.parentId.indexOf('/') + 1);
  return list;
}

export function machinesOnline(v: View): number {
  return [...v.nodes.values()].filter((n) => n.online).length;
}

export interface ApprovalModel {
  id: string; // hub-wide key `<node>/<local id>`
  node: string;
  localId: string;
  agentId: string;
  agentName: string; // the agent's display name; empty when this window does not know the agent (never an id)
  machineName?: string; // the machine's display name; empty or missing when unknown (never a node id)
  action: string;
  detail: string;
  expiresAtMs: number;
  start?: StartRequest; // ADR-0057: what an `agent.start` request asks for
  skip?: SkipRequest; // ADR-0090 Amendment A: what an `agent.skip_permissions` request says, read strictly
  skipUnknown?: boolean; // the action is that kind but the card is not the shape this window expects: nothing is signed
  always?: AlwaysOffer; // ADR-0101: the request offers an answer the agent remembers (and maybe only that)
}

// An agent's request to start another (ADR-0057), read from the approval's detail. Every field is the asking agent's text: shown, never
// interpreted. The detail itself is what the decision is signed over, so it is kept as it came.
export interface StartRequest { agent: string; agentKind: string; node: string; kind: string; folder: string; prompt: string; inside: boolean; worktree: boolean; depth: number; grantAllowed: boolean; settings: StartSetting[]; options: StartOption[]; remote?: RemoteStart }
// ADR-0119: a start another machine asked this one for (`agent.start.remote`): the asking machine (the hub's word) and its agent and that
// agent's kind (the asking machine's word).
export interface RemoteStart { originNode: string; originAgent: string; originAgentKind: string }
// ADR-0066: a launch setting the request asks for (`label` is the machine's Fluent id for it, `value` the agent's text: shown, never interpreted).
export interface StartSetting { id: string; label: string; value: string }

/** ADR-0066: the `settings` of a start request's detail, read the way the node reads them (a list of `{id, label, value}`; anything that is
 * not a string is an empty string, so a request is signed over exactly what it shows). */
export function startSettings(raw: unknown): StartSetting[] {
  if (!Array.isArray(raw)) return [];
  return raw.map((x) => {
    const o = (typeof x === 'object' && x !== null ? x : {}) as Record<string, unknown>;
    const str = (k: string) => (typeof o[k] === 'string' ? (o[k] as string) : '');
    return { id: str('id'), label: str('label'), value: str('value') };
  });
}

/** ADR-0119 decision 10: a helper another machine asked for hangs under its parent there (the hub-wide key `<machine>/<agent>`); its own
 *  `parent_agent_id` stays empty so an older window shows it at the top level, never under a wrong local agent. */
function remoteParent(node: string | null | undefined, agent: string | null | undefined): string | undefined {
  return node && agent && !node.includes('/') && !agent.includes('/') ? `${node}/${agent}` : undefined;
}

export function startRequest(action: string, detail: string): StartRequest | undefined {
  if (action === 'agent.start.remote') {
    try {
      const d = JSON.parse(detail) as Record<string, unknown>;
      const s = (k: string) => (typeof d[k] === 'string' ? (d[k] as string) : '');
      // The folder is a full path on this machine, named by the asker: never "outside" anything here, and the depth is the asker's.
      return { agent: s('origin_agent'), agentKind: s('origin_agent_kind'), node: s('node'), kind: s('kind'), folder: s('folder'), prompt: s('prompt'), inside: true, worktree: d.worktree === true, depth: typeof d.depth === 'number' ? d.depth : 0, grantAllowed: d.grant_allowed === true, settings: startSettings(d.settings), options: detailLaunchOptions(detail), remote: { originNode: s('origin_node'), originAgent: s('origin_agent'), originAgentKind: s('origin_agent_kind') } };
    } catch {
      return undefined;
    }
  }
  if (action !== 'agent.start') return undefined;
  try {
    const d = JSON.parse(detail) as Record<string, unknown>;
    const s = (k: string) => (typeof d[k] === 'string' ? (d[k] as string) : '');
    // `grant_allowed` (a scoped grant may be given: not for a shell or plain terminal program); a node that does not say so offers none.
    return { agent: s('agent'), agentKind: s('agent_kind'), node: s('node'), kind: s('kind'), folder: s('folder'), prompt: s('prompt'), inside: d.inside === true, worktree: d.worktree === true, depth: typeof d.depth === 'number' ? d.depth : 1, grantAllowed: d.grant_allowed === true, settings: startSettings(d.settings), options: detailLaunchOptions(detail) };
  } catch {
    return undefined;
  }
}

const skipFields = (r: ReturnType<typeof skipOf>): Pick<ApprovalModel, 'skip' | 'skipUnknown'> => (r === undefined ? {} : 'ok' in r ? { skip: r.ok } : { skipUnknown: true });

// ADR-0090 Amendment A: a card for a switch to a mode that skips permissions belongs to the agent's run; when the agent ends the node
// withdraws it, and the window drops it at once too, so a stale card is never left to answer.
function dropSkipCards(v: View, agentKey: string): void {
  for (const [id, a] of v.approvals) if (a.agentId === agentKey && a.action === SKIP_ACTION) v.approvals.delete(id);
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
        agentName: names.get(a.agentId) ?? '',
        machineName: v.nodes.get(at < 0 ? '' : id.slice(0, at))?.hostname ?? '',
        action: a.action,
        detail: a.detail,
        expiresAtMs: a.expiresAtMs,
        start: startRequest(a.action, a.detail),
        ...(a.scopes ? (() => { const o = alwaysOffer(a.action, a.scopes, a.alwaysLabel ?? '', a.alwaysOptionId ?? ''); return o ? { always: o } : {}; })() : {}),
        ...skipFields(skipOf(a.action, a.detail, at < 0 ? '' : id.slice(0, at), a.agentId)),
      };
    })
    .sort((a, b) => a.expiresAtMs - b.expiresAtMs);
}

// ADR-0073: a machine not seen for a day is shown as old (the hub's `agora hub machines` uses the same threshold).
export const OLD_AFTER_MS = 24 * 60 * 60 * 1000;

export function isOld(m: { online: boolean; lastSeenMs: number }, now: number): boolean {
  return !m.online && now - m.lastSeenMs >= OLD_AFTER_MS;
}

// ADR-0073: the view as people see it: a forgotten machine and everything that belongs to it (its folders, worktrees, terminals,
// approvals and agents) are left out. A new object either way, so React sees the change.
export function shown(v: View): View {
  const hidden = new Set([...v.nodes].filter(([, n]) => n.forgotten).map(([id]) => id));
  if (hidden.size === 0) return { ...v };
  const under = (key: string) => hidden.has(key.split('/')[0] ?? '');
  const keep = <T,>(m: Map<string, T>, ok: (k: string, x: T) => boolean) => new Map([...m].filter(([k, x]) => ok(k, x)));
  return {
    ...v,
    nodes: keep(v.nodes, (id) => !hidden.has(id)),
    checkouts: keep(v.checkouts, (k, c) => !under(k) && !hidden.has(c.nodeId)),
    workspaces: keep(v.workspaces, (k, w) => !under(k) && !hidden.has(w.nodeId)),
    worktrees: keep(v.worktrees, (k) => !under(k)),
    agents: keep(v.agents, (k, a) => !under(k) && !hidden.has(a.nodeId)),
    agentWorkspaces: keep(v.agentWorkspaces, (k) => !under(k)),
    approvals: keep(v.approvals, (k) => !under(k)),
    withTerminal: new Set([...v.withTerminal].filter((k) => !under(k))),
    trusted: v.trusted && keep(v.trusted, (id) => !hidden.has(id)),
    quota: v.quota && keep(v.quota, (node) => !hidden.has(node)),
  };
}

/** ADR-0091: the plan-limit observations the window holds, for the Usage panel, the cards and the start window. */
export function quotaObs(v: View): QuotaObs[] {
  return [...(v.quota?.values() ?? [])].flatMap((m) => [...m.values()]).map((q) => ({ machineId: q.nodeId, tool: q.tool, window: q.window, usedPercent: q.usedPercent, resetsAtMs: q.resetsAtMs, observedAtMs: q.observedAtMs }));
}

export interface MachineModel { id: string; name: string; os: string; online: boolean; agentsRunning: number; lastSeenMs: number; memoryBytes?: number; build?: string; terminalsOpen?: number; binary?: string }

// Online machines first, then by name.
export function machines(v: View): MachineModel[] {
  return [...v.nodes.entries()]
    .map(([id, n]) => ({ id, name: visible(n.hostname || id), os: visible(n.os), online: n.online, agentsRunning: n.agentsRunning, lastSeenMs: n.lastEventMs, memoryBytes: n.memoryBytes, build: n.build === undefined ? undefined : visible(n.build), terminalsOpen: n.terminalsOpen, binary: n.binary === undefined ? undefined : visible(n.binary) }))
    .sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name));
}

export interface ProjectChoice { node: string; projectId: string; label: string; path?: string }

// Where an agent can be started: every project checked out on a machine that is online.
export function projectChoices(v: View): ProjectChoice[] {
  const choices = new Map<string, ProjectChoice>();
  for (const c of [...v.checkouts.values(), ...v.workspaces.values()]) {
    if (v.nodes.get(c.nodeId)?.online) choices.set(`${c.nodeId}/${c.path}`, { node: c.nodeId, projectId: c.projectId, path: c.path,
      label: visible(`${v.nodes.get(c.nodeId)?.hostname || c.nodeId} · ${lastSegment(c.path) || c.projectId}`) });
  }
  return [...choices.values()].sort((a, b) => a.label.localeCompare(b.label));
}

// `via`: the machine whose word alone says who sent it, when that is not the sender's machine (see `MessageV.via`). It is the
// machine's id, which the hub checked against its certificate, never the host name the machine reports for itself: a node could
// call itself after the sender's machine.
// `fromId` is the sender's key, so the team chat can draw its avatar; `toMachine` names the recipient's machine on a message between machines.
export interface MessageModel { from: string; to: string; body: string; atMs: number; crossMachine: boolean; via?: string; fromId?: string; toMachine?: string }

// The newest messages first, with agent names instead of ids. A message between two machines says so.
export function recentMessages(v: View, limit = 5): MessageModel[] {
  const byId = new Map(tiles(v).map((t) => [t.id, t]));
  const label = (id: string) => byId.get(id)?.name ?? id;
  const machine = (id: string) => id.split('/')[0];
  return v.messages.slice(-limit).reverse().map((m) => {
    const crossMachine = m.to.includes('/') && machine(m.from) !== machine(m.to);
    return {
      from: label(m.from), to: label(m.to), body: m.body, atMs: m.atMs, crossMachine, fromId: m.from,
      ...(crossMachine ? { toMachine: byId.get(m.to)?.machine ?? machine(m.to) } : {}),
      ...(m.via ? { via: m.via } : {}),
    };
  });
}

// `id` is the board's key (`<project>/<task>`); `taskId` is the id the board and the node know.
interface SubtaskProgress { accepted: number; total: number }
export interface BoardTaskModel { id: string; taskId?: string; title: string; status: 'open' | 'claimed' | 'review' | 'done'; review?: TaskReviewModel; owner: string; ownerState?: UiState; ownerSkips?: boolean; held?: string[]; handoffs?: HandoffDeliveryModel[]; parent?: { title: string }; subtasks?: SubtaskProgress }
export interface BoardLeaseModel { path: string; holder: string; expiresAtMs: number }
export interface BoardFolderModel { nodeId: string; machine: string; path: string }
export interface BoardModel { projectId: string; project: string; reviewEnabled?: boolean; folders: BoardFolderModel[]; tasks: BoardTaskModel[]; leases: BoardLeaseModel[] }

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
      by.set(id, (b = { projectId: id, project: context?.project || id, ...(v.reviewProjects.has(id) ? { reviewEnabled: true } : {}), folders, tasks: [], leases: [] }));
    }
    return b;
  };
  const heldBy = new Map<string, string[]>();
  for (const l of v.leases.values()) { const list = heldBy.get(l.agentId); if (list) list.push(l.path); else heldBy.set(l.agentId, [l.path]); }
  const handoffsByTask = new Map<string, HandoffDeliveryModel[]>();
  for (const delivery of v.handoffDeliveries.values()) {
    const row = { ...delivery, receiver: label(delivery.receiver) };
    const list = handoffsByTask.get(delivery.taskKey);
    if (list) list.push(row);
    else handoffsByTask.set(delivery.taskKey, [row]);
  }
  const subtasks = new Map<string, SubtaskProgress>();
  for (const [id, task] of v.tasks) {
    if (!task.parentTaskId) continue;
    const parentKey = `${task.projectId}/${task.parentTaskId}`;
    const progress = subtasks.get(parentKey) ?? { accepted: 0, total: 0 };
    progress.total += 1;
    if (v.reviews.get(id)?.state === 'accepted') progress.accepted += 1;
    subtasks.set(parentKey, progress);
  }
  for (const [id, t] of v.tasks) {
    const owner = t.ownerId ? agents.get(t.ownerId) : undefined;
    const review = v.reviews.get(id);
    const parent = t.parentTaskId ? v.tasks.get(`${t.projectId}/${t.parentTaskId}`) : undefined;
    const progress = subtasks.get(id);
    board(t.projectId).tasks.push({ id, taskId: id.slice(t.projectId.length + 1), title: t.title, status: review?.state === 'pending' ? 'review' : t.done ? 'done' : t.ownerId ? 'claimed' : 'open', ...(review ? { review } : {}), owner: t.ownerId ? label(t.ownerId) : '', ownerState: owner?.state, ownerSkips: owner?.skipsPermissions, held: t.ownerId ? heldBy.get(t.ownerId) ?? [] : [], handoffs: handoffsByTask.get(id)?.reverse(), ...(parent ? { parent: { title: parent.title } } : {}), ...(progress ? { subtasks: progress } : {}) });
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

// Estimated spend per project (the folder name of its checkout or selected folder), biggest first. Only agents the hub had a price for
// count; `perAgent` is the hub's answer keyed by agent. Agents whose project is not known add to no row.
export function costByProject(v: View, perAgent: ReadonlyMap<string, number>): ProjectCost[] {
  const byProject = new Map<string, ProjectCost>();
  for (const [id, a] of v.agents) {
    const micro = perAgent.get(id);
    if (micro === undefined) continue;
    // F1420: an agent started in a folder belongs to that folder's project, as its tile says; one in a worktree, to its checkout's.
    const ws = v.workspaces.get(v.agentWorkspaces.get(id) ?? '');
    const checkout = ws ? undefined : v.checkouts.get(v.worktrees.get(a.worktreeId)?.checkoutId ?? '');
    const project = ws ? { projectId: ws.projectId, path: ws.gitRoot || ws.path } : checkout;
    if (!project) continue;
    const row = byProject.get(project.projectId) ?? { label: lastSegment(project.path) || project.projectId, microUsd: 0 };
    row.microUsd += micro;
    byProject.set(project.projectId, row);
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
