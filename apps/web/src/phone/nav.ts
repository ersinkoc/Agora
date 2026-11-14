// ADR-0067 rung 2: where a phone-sized window is, from the address. Pure: the screens draw what this decides.
import { type AgentTileModel, machineKey, sortFleet } from '../model';
import type { ApprovalModel, MachineModel } from '../net/view';

// A phone: a narrow window, or a short one on a touch screen (a phone held sideways). A tablet and every desktop window keep the
// full window.
export const PHONE_QUERY = '(max-width: 699px), (max-height: 500px) and (pointer: coarse)';

// The phone's screens: the list (what needs you, then the machines and their agents) and one agent. Anything else in the address is
// a page the window already has (machines, this device, audit, the gallery).
export type PhoneScreen = { kind: 'home' } | { kind: 'agent'; id: string } | { kind: 'approval'; id: string } | { kind: 'theme' } | { kind: 'page' };

function decode(s: string): string | undefined {
  try { return decodeURIComponent(s); } catch { return undefined; }
}

export function phoneScreen(hash: string): PhoneScreen {
  if (hash === '' || hash === '#' || hash === '#/') return { kind: 'home' };
  if (hash.startsWith('#/agent/')) { const id = decode(hash.slice('#/agent/'.length)); return id ? { kind: 'agent', id } : { kind: 'home' }; }
  // ADR-0046: a notice about an approval links to `#/approvals/<node>%2F<id>`.
  if (hash.startsWith('#/approvals/')) { const id = decode(hash.slice('#/approvals/'.length)); return id ? { kind: 'approval', id } : { kind: 'home' }; }
  // The theme editor is a panel of the full window (ADR-0068); a phone shows it as a screen of its own.
  if (hash === '#/theme') return { kind: 'theme' };
  if (/^#\/(machines|device|audit|gallery|demo)$/.test(hash)) return { kind: 'page' };
  return { kind: 'home' };
}

// Where Back goes: an agent, the theme editor and every page go back to the list.
export function backOf(screen: PhoneScreen): string | undefined {
  return screen.kind === 'home' ? undefined : '#/';
}

export const agentHash = (id: string) => `#/agent/${encodeURIComponent(id)}`;

// A notice's link opens the agent that asks, once the window knows the approval. One already answered (or unknown) opens the list,
// where whatever still waits is first. `undefined`: nothing to do yet (the fleet has not arrived).
export function approvalRedirect(screen: PhoneScreen, approvals: readonly ApprovalModel[], loaded: boolean): string | undefined {
  if (screen.kind !== 'approval') return undefined;
  const a = approvals.find((x) => x.id === screen.id);
  if (a) return agentHash(a.agentId);
  return loaded ? '#/' : undefined;
}

// What needs you, in the order to answer it: requests to approve (soonest to expire first), then agents waiting on you that have no
// request open (longest waiting first).
export type NeedsItem = { kind: 'approval'; approval: ApprovalModel; agent?: AgentTileModel } | { kind: 'agent'; agent: AgentTileModel };
export function needsYou(approvals: readonly ApprovalModel[], fleet: readonly AgentTileModel[]): NeedsItem[] {
  const byId = new Map(fleet.map((a) => [a.id, a]));
  const asks = [...approvals].sort((a, b) => a.expiresAtMs - b.expiresAtMs).map((approval): NeedsItem => ({ kind: 'approval', approval, agent: byId.get(approval.agentId) }));
  const asking = new Set(approvals.map((x) => x.agentId));
  const waiting = fleet.filter((a) => a.state === 'needs-you' && !a.closed && !asking.has(a.id))
    .map((a, i) => [a, i] as const).sort(([a, i], [b, j]) => (a.since ?? Infinity) - (b.since ?? Infinity) || i - j)
    .map(([agent]): NeedsItem => ({ kind: 'agent', agent }));
  return [...asks, ...waiting];
}

// The machines with their agents, most urgent first inside each; online machines first, then by name. A machine the fleet names but the hub has not
// listed (demo data, a machine that left) still gets its group.
export interface MachineGroup { id: string; name: string; online: boolean; agents: AgentTileModel[] }
export function byMachine(fleet: readonly AgentTileModel[], machines: readonly MachineModel[]): MachineGroup[] {
  const groups = new Map<string, MachineGroup>(machines.map((m) => [m.id, { id: m.id, name: m.name, online: m.online, agents: [] }]));
  // Closed agents come last in their machine, where they stay until cleared.
  for (const a of [...sortFleet(fleet.filter((x) => !x.closed)), ...fleet.filter((x) => x.closed)]) {
    const key = machineKey(a);
    let g = groups.get(key) ?? [...groups.values()].find((x) => x.name === key);
    if (!g) { g = { id: key, name: a.machine || key, online: true, agents: [] }; groups.set(key, g); }
    g.agents.push(a);
  }
  return [...groups.values()].sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name));
}
