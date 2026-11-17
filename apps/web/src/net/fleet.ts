// The fleet console's data (docs/ux/visual-language.md, "Glance"): what needs you, what works, what finished, what failed and which
// machines are online, derived from the state the window already has. Pure: no DOM, no clock (the caller passes `now`), no state of its
// own. Everything here is one pass over the list (plus one sort), so the console memoises it on the list and a 500-agent fleet costs
// about as much as drawing one row.
import { type AgentTileModel, type UiState, endedWithError, machineKey } from '../model';
import { detailText } from './tool';
import { type ApprovalModel, type MachineModel, visible } from './view';

// The five questions of the header strip. `needs-you`, `working`, `finished` and `failed` filter the tiles; machines link to their page.
export type FleetFilter = 'needs-you' | 'working' | 'finished' | 'failed';
export const FLEET_FILTERS: readonly FleetFilter[] = ['needs-you', 'working', 'finished', 'failed'];

// An agent whose program was running when its machine stopped is neither finished nor failed: it has its own words (ADR-0060).
const stopped = (a: AgentTileModel) => !!a.closed && !!a.interrupted;

// The one bucket an agent counts in. Idle and unknown agents have none: the strip answers five questions, not every state.
export function bucketOf(a: AgentTileModel): FleetFilter | undefined {
  if (endedWithError(a)) return 'failed';
  if (a.closed) return stopped(a) ? undefined : 'finished';
  if (a.state === 'needs-you') return 'needs-you';
  if (a.state === 'working') return 'working';
  if (a.state === 'finished') return 'finished';
  return undefined;
}

// Most urgent first: needs you, working, failed (it wants a look), idle, finished, unknown. One rank per agent, so a sort never calls
// back into the model.
const RANK: Record<UiState, number> = { 'needs-you': 0, working: 1, idle: 3, finished: 4, unknown: 5 };
export const rankOf = (a: AgentTileModel): number => (endedWithError(a) ? 2 : a.closed && a.state === 'finished' ? 4 : RANK[a.state]);

// Stable: agents of one rank keep the order they came in, so a tile never jumps when something else changes.
export function sortByUrgency(list: readonly AgentTileModel[]): AgentTileModel[] {
  const keyed = list.map((a, i) => ({ a, r: rankOf(a), i }));
  keyed.sort((x, y) => x.r - y.r || x.i - y.i);
  return keyed.map((k) => k.a);
}

// One request, or one agent that waits without a request open (it asks in its terminal).
export type AttentionItem =
  | { kind: 'approval'; key: string; approval: ApprovalModel; agent?: AgentTileModel; sinceMs?: number }
  | { kind: 'agent'; key: string; agent: AgentTileModel; sinceMs?: number };

// What needs the user, most urgent first: requests that will lapse soonest, then requests without a known expiry, then agents waiting
// in their terminals, each group the longest-waiting first. An agent with a request open is one thing that needs you, not two.
export function attentionList(approvals: readonly ApprovalModel[], fleet: readonly AgentTileModel[]): AttentionItem[] {
  const byId = new Map<string, AgentTileModel>();
  for (const a of fleet) byId.set(a.id, a);
  const asking = new Set<string>();
  const asks: { item: AttentionItem; expires: number; since: number; i: number }[] = [];
  approvals.forEach((approval, i) => {
    const agent = byId.get(approval.agentId);
    asking.add(approval.agentId);
    asks.push({ item: { kind: 'approval', key: approval.id, approval, agent, sinceMs: agent?.since }, expires: approval.expiresAtMs > 0 ? approval.expiresAtMs : Infinity, since: agent?.since ?? Infinity, i });
  });
  asks.sort((x, y) => x.expires - y.expires || x.since - y.since || x.i - y.i);
  const waiting: { item: AttentionItem; since: number; i: number }[] = [];
  fleet.forEach((a, i) => {
    if (a.state === 'needs-you' && !a.closed && !asking.has(a.id)) waiting.push({ item: { kind: 'agent', key: a.id, agent: a, sinceMs: a.since }, since: a.since ?? Infinity, i });
  });
  waiting.sort((x, y) => x.since - y.since || x.i - y.i);
  return [...asks.map((x) => x.item), ...waiting.map((x) => x.item)];
}

// Approve or Deny in the row only where the request is a plain yes or no that needs no reading, and the row shows ALL of it: no start
// request (it carries a folder, a prompt and settings), no settings, one line each of action and detail, no structured text the agent wrote,
// no character that is invisible or changes how the text looks, and no more than INLINE_MAX characters in all (the text as the row writes
// it, "action · detail"). The row never cuts or clamps what it shows for such a request, it grows to hold it. Anything else is opened in
// the full card, where it is read in full before it is answered: a decision is never taken on text that was cut or hidden.
export const INLINE_MAX = 120;
export const askLine = (action: string, detail: string): string => (detail ? `${action} · ${detail}` : action);
export function decidableInline(a: ApprovalModel): boolean {
  if (a.start) return false;
  const oneLine = (s: string) => !/[\r\n]/.test(s) && visible(s) === s;
  if (!a.action.trim() || !oneLine(a.action)) return false;
  if (a.detail && (!oneLine(a.detail) || detailText(a.detail) !== a.detail)) return false;
  return askLine(a.action, a.detail).length <= INLINE_MAX;
}

// Many agents asking for the same thing are one row with a count (a stack of identical Allow buttons invites approving by reflex): each is
// reviewed on its own. The agent's kind is the first part of its name ("claude · amber").
export const SAME_ASK_MIN = 6;
export type AttentionRow = { kind: 'one'; item: AttentionItem } | { kind: 'many'; key: string; items: AttentionItem[]; agentKind: string; ask: string };
const kindOf = (name: string): string => name.split(' · ')[0] ?? name;
function sameKey(it: AttentionItem): { key: string; agentKind: string; ask: string } {
  if (it.kind === 'approval') {
    const agentKind = kindOf(it.agent?.name || it.approval.agentName || '');
    const ask = it.approval.start ? '\u0000start' : askLine(it.approval.action, it.approval.detail);
    return { key: `${agentKind}\u0001${ask}`, agentKind, ask };
  }
  const agentKind = kindOf(it.agent.name);
  return { key: `${agentKind}\u0001\u0000terminal`, agentKind, ask: '' };
}
export function collapseAttention(items: readonly AttentionItem[], min = SAME_ASK_MIN): AttentionRow[] {
  const keyed = items.map((it) => ({ it, ...sameKey(it) }));
  const count = new Map<string, number>();
  for (const k of keyed) count.set(k.key, (count.get(k.key) ?? 0) + 1);
  const out: AttentionRow[] = [];
  const seen = new Map<string, Extract<AttentionRow, { kind: 'many' }>>();
  for (const k of keyed) {
    if ((count.get(k.key) ?? 0) < min) { out.push({ kind: 'one', item: k.it }); continue; }
    const g = seen.get(k.key);
    if (g) g.items.push(k.it);
    else { const row = { kind: 'many' as const, key: k.key, items: [k.it], agentKind: k.agentKind, ask: k.ask }; seen.set(k.key, row); out.push(row); }
  }
  return out;
}

export interface FleetCounts {
  needs: number; // things that need the user (requests, plus agents waiting without one)
  working: number;
  finished: number;
  failed: number;
  agents: number; // every agent counted, closed ones included
  machinesOnline: number;
  machinesTotal: number;
}

export function fleetCounts(fleet: readonly AgentTileModel[], approvals: readonly ApprovalModel[], machines: readonly MachineModel[]): FleetCounts {
  const c: FleetCounts = { needs: 0, working: 0, finished: 0, failed: 0, agents: fleet.length, machinesOnline: 0, machinesTotal: machines.length };
  // One row per request, plus one per agent waiting in its terminal with no request open: the same number as the attention list.
  const asking = new Set<string>();
  for (const x of approvals) asking.add(x.agentId);
  c.needs = approvals.length;
  for (const a of fleet) {
    const b = bucketOf(a);
    if (b === 'needs-you') { if (!asking.has(a.id)) c.needs++; }
    else if (b === 'working') c.working++;
    else if (b === 'finished') c.finished++;
    else if (b === 'failed') c.failed++;
  }
  for (const m of machines) if (m.online) c.machinesOnline++;
  return c;
}

export const matchesFilter = (a: AgentTileModel, f: FleetFilter | undefined): boolean => !f || bucketOf(a) === f;

export type GroupBy = 'machine' | 'project';
export interface FleetGroup {
  key: string;
  name: string;
  machine?: MachineModel; // for a machine group: its health, to say "offline" and since when
  online: boolean;
  agents: AgentTileModel[];
}

// Tiles grouped by machine or by project, most urgent first inside each group; groups with something that needs the user first, then
// the rest by name (online machines before offline ones). `sorted` must already be in urgency order (`sortByUrgency`): groups are filled
// in one pass and keep it. A machine group is made for every machine the hub lists, even with no agent in it (so an offline machine and a
// new one are seen), but not while a filter is on: then only groups with a match are shown.
export function groupFleet(sorted: readonly AgentTileModel[], machines: readonly MachineModel[], by: GroupBy, filter?: FleetFilter): FleetGroup[] {
  const groups = new Map<string, FleetGroup>();
  const byMachineName = new Map<string, FleetGroup>();
  if (by === 'machine') {
    for (const m of machines) {
      const g: FleetGroup = { key: m.id, name: m.name, machine: m, online: m.online, agents: [] };
      groups.set(m.id, g);
      byMachineName.set(m.name, g);
    }
  }
  for (const a of sorted) {
    if (!matchesFilter(a, filter)) continue;
    let g: FleetGroup | undefined;
    if (by === 'machine') {
      const key = machineKey(a);
      g = groups.get(key) ?? byMachineName.get(key);
      if (!g) { g = { key, name: a.machine || key, online: true, agents: [] }; groups.set(key, g); }
    } else {
      const key = a.projectId ?? (a.project ? `name:${a.project}` : '');
      g = groups.get(key);
      if (!g) { g = { key, name: a.project || '', online: true, agents: [] }; groups.set(key, g); }
    }
    g.agents.push(a);
  }
  const list = [...groups.values()].filter((g) => !filter || g.agents.length > 0);
  const urgency = (g: FleetGroup) => (g.agents[0] ? rankOf(g.agents[0]) : 9);
  // The project without a name comes last whatever its agents are doing: it is a leftover, not a place.
  const leftover = (g: FleetGroup) => by === 'project' && g.key === '';
  return list.sort((x, y) => Number(leftover(x)) - Number(leftover(y)) || Number(y.online) - Number(x.online) || urgency(x) - urgency(y) || x.name.localeCompare(y.name));
}

// The last line of an agent's terminal that says something: its one-line current activity. Empty when the client has no preview of the
// terminal (the hub does not send one yet), and then the tile shows what it does have instead; nothing is invented.
export function lastLine(preview: string): string {
  if (!preview) return '';
  const lines = preview.split('\n');
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = (lines[i] as string).trim();
    if (line) return line;
  }
  return '';
}

// Everything the console draws, in one memoisable value.
export interface FleetView {
  sorted: AgentTileModel[];
  counts: FleetCounts;
  attention: AttentionItem[];
}
export function fleetView(fleet: readonly AgentTileModel[], approvals: readonly ApprovalModel[], machines: readonly MachineModel[]): FleetView {
  return { sorted: sortByUrgency(fleet), counts: fleetCounts(fleet, approvals, machines), attention: attentionList(approvals, fleet) };
}

// What the page shows when it has no fleet to draw (the choice of "teach", "wait" or "say it is down"):
//  - `connecting`: the link is still coming up and nothing is known, so it must not say "no machines" yet;
//  - `offline`: the hub cannot be reached and nothing is known;
//  - `empty`: a hub with no machine and no agent;
//  - `first-run`: machines are connected, no agent has been started;
//  - undefined: there is a fleet to draw.
export type FleetEmptyKind = 'connecting' | 'offline' | 'empty' | 'first-run';
export function emptyKind(link: 'connecting' | 'live' | 'offline', agents: number, machines: number): FleetEmptyKind | undefined {
  if (agents > 0) return undefined;
  if (machines === 0) return link === 'connecting' ? 'connecting' : link === 'offline' ? 'offline' : 'empty';
  return link === 'live' ? 'first-run' : undefined;
}

// A short span for "in this state for": "40 s", "12 min", "2 h 5 min", "3 d". The marks are the same in every language, like KB.
export function span(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
  return `${Math.floor(h / 24)} d`;
}
