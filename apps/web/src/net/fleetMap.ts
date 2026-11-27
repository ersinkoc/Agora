// ADR-0129: the live map of the fleet, as plain data. A fixed radial layout: the hub in the middle, the machines on a ring around it
// (sorted by name, so the same fleet always draws the same way), each machine's agents on an arc beyond it, a helper one step further
// out from its parent. Pure: the same machines and agents give the same picture; render reads it and never changes it.
import type { AgentTileModel, UiState } from '../model';
import type { MachineModel } from './view';

/** Coordinates are in a square from -EXTENT to EXTENT, the hub at 0,0. */
export const EXTENT = 520;
const MACHINE_RING = 230;
const AGENT_RING = 360;
const HELPER_STEP = 70;
/** Agents drawn per machine; the rest are counted on the machine ("+N"). */
export const AGENTS_PER_MACHINE = 24;

export interface MapPoint { x: number; y: number }
export interface MapMachine extends MapPoint { id: string; name: string; online: boolean; angle: number; hidden: number }
export interface MapAgent extends MapPoint { id: string; name: string; machine: string; state: UiState; accent: string; kind?: string; parent?: string; closed?: boolean }
export interface MapEdge { from: MapPoint; to: MapPoint; kind: 'link' | 'runs' | 'helper'; dim: boolean }
export interface FleetMap { hub: MapPoint; machines: MapMachine[]; agents: MapAgent[]; edges: MapEdge[] }

const polar = (angle: number, r: number): MapPoint => ({ x: Math.round(Math.cos(angle) * r * 10) / 10, y: Math.round(Math.sin(angle) * r * 10) / 10 });
const byName = (a: { name: string; id: string }, b: { name: string; id: string }) => a.name.localeCompare(b.name) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/** The machine an agent runs on: its enrolled id, else the machine named as it is (an older hub). */
function machineOf(a: Pick<AgentTileModel, 'machineId' | 'machine' | 'id'>, machines: readonly MachineModel[]): string | undefined {
  if (a.machineId && machines.some((m) => m.id === a.machineId)) return a.machineId;
  const byLabel = machines.find((m) => m.name === a.machine);
  if (byLabel) return byLabel.id;
  const prefix = a.id.split('/')[0];
  return machines.some((m) => m.id === prefix) ? prefix : undefined;
}

export function fleetMap(machines: readonly MachineModel[], agents: readonly AgentTileModel[]): FleetMap {
  const hub = { x: 0, y: 0 };
  const sorted = [...machines].sort(byName);
  const n = Math.max(1, sorted.length);
  const slice = (2 * Math.PI) / n;
  const out: FleetMap = { hub, machines: [], agents: [], edges: [] };
  const live = agents.filter((a) => !a.closed);
  sorted.forEach((m, i) => {
    const angle = -Math.PI / 2 + i * slice;
    const mine = live.filter((a) => machineOf(a, machines) === m.id);
    const ids = new Set(mine.map((a) => a.id));
    // Top-level agents (no parent here) on an arc centred on the machine; helpers follow their parent outwards.
    const tops = mine.filter((a) => !a.parentId || !ids.has(a.parentId)).sort(byName);
    const shown = tops.slice(0, AGENTS_PER_MACHINE);
    const hiddenTops = tops.length - shown.length;
    const point = polar(angle, MACHINE_RING);
    const machine: MapMachine = { id: m.id, name: m.name, online: m.online, angle, hidden: hiddenTops, ...point };
    out.machines.push(machine);
    out.edges.push({ from: hub, to: point, kind: 'link', dim: !m.online });
    const arc = Math.min(slice * 0.8, Math.PI * 0.9);
    const step = shown.length > 1 ? arc / (shown.length - 1) : 0;
    const place = (a: AgentTileModel, ang: number, r: number, parent: MapPoint, parentId: string | undefined, depth: number) => {
      const p = polar(ang, r);
      out.agents.push({ id: a.id, name: a.name, machine: m.id, state: a.state, accent: a.accent, kind: a.kind, parent: parentId, ...p });
      out.edges.push({ from: parent, to: p, kind: parentId ? 'helper' : 'runs', dim: !m.online });
      if (depth >= 3) return; // deeper helpers are counted with their parent, not drawn
      const kids = mine.filter((c) => c.parentId === a.id).sort(byName);
      const spread = Math.min(step || arc / 3, 0.3);
      kids.forEach((c, k) => place(c, ang + (k - (kids.length - 1) / 2) * (spread / Math.max(1, kids.length)), r + HELPER_STEP, p, a.id, depth + 1));
    };
    shown.forEach((a, k) => place(a, angle - (shown.length > 1 ? arc / 2 : 0) + k * step, AGENT_RING, point, undefined, 1));
  });
  return out;
}

/** The way a moving item travels between two agents: straight on one machine, through the hub between two. */
export function route(map: FleetMap, fromId: string, toId: string): MapPoint[] | undefined {
  const from = map.agents.find((a) => a.id === fromId);
  const to = map.agents.find((a) => a.id === toId);
  if (!from || !to) return undefined;
  if (from.machine === to.machine) return [from, to];
  const fm = map.machines.find((m) => m.id === from.machine);
  const tm = map.machines.find((m) => m.id === to.machine);
  return fm && tm ? [from, fm, map.hub, tm, to] : [from, map.hub, to];
}

/** The way from the hub to an agent (a task or a handoff the hub gives it); `undefined` when the agent is not drawn. `id` may be the
 *  hub-wide key or a machine's own agent id. */
export function fromHub(map: FleetMap, id: string): MapPoint[] | undefined {
  const to = map.agents.find((a) => a.id === id || a.id.endsWith(`/${id}`));
  if (!to) return undefined;
  const m = map.machines.find((x) => x.id === to.machine);
  return m ? [map.hub, m, to] : [map.hub, to];
}

/** A task given to an agent, or a handoff delivered to one, as the map moves it: a stable key and where it goes. */
export interface MapTaskSignal { key: string; to: string; words: string }

/** What the boards say was given to whom: each claimed task once per owner, each delivered handoff once. */
export function taskSignals(boards: readonly { tasks: readonly { id: string; title: string; ownerId?: string; handoffs?: readonly { id: string; receiver: string; state: string }[] }[] }[]): MapTaskSignal[] {
  const out: MapTaskSignal[] = [];
  for (const b of boards) {
    for (const t of b.tasks) {
      if (t.ownerId) out.push({ key: `task\u0000${t.id}\u0000${t.ownerId}`, to: t.ownerId, words: t.title });
      for (const h of t.handoffs ?? []) if (h.state === 'delivered' && h.receiver) out.push({ key: `handoff\u0000${h.id}`, to: h.receiver, words: t.title });
    }
  }
  return out;
}

/** Where along a path a moving item is at `t` (0..1), by length. */
export function along(path: readonly MapPoint[], t: number): MapPoint {
  if (path.length === 1) return path[0] as MapPoint;
  const lengths = path.slice(1).map((p, i) => Math.hypot(p.x - (path[i] as MapPoint).x, p.y - (path[i] as MapPoint).y));
  const total = lengths.reduce((a, b) => a + b, 0) || 1;
  let left = Math.min(1, Math.max(0, t)) * total;
  for (let i = 0; i < lengths.length; i++) {
    const len = lengths[i] as number;
    const a = path[i] as MapPoint;
    const b = path[i + 1] as MapPoint;
    if (left <= len || i === lengths.length - 1) {
      const f = len ? Math.min(1, left / len) : 1;
      return { x: a.x + (b.x - a.x) * f, y: a.y + (b.y - a.y) * f };
    }
    left -= len;
  }
  return path[path.length - 1] as MapPoint;
}
