import type { AgentTileModel } from '../model';
import { contextPercent } from '../components/sessionChoice';

// The Usage panel: what the window already knows about how much each agent has been used, without money. Subscriptions, not
// pay-per-token, are how most people pay, so a dollar figure says little; tokens, context and time say what happened.
// Pure, so it is tested without a page. No money anywhere: not an estimate, not what an agent says it cost.

export interface UsageAgent { id: string; name: string; accent: string; kind: string; tokens: number; contextPct?: number; activeMs?: number; running: boolean }
export interface UsageGroup { key: string; machine: string; project: string; tokens: number; activeMs: number; agents: UsageAgent[] }
export interface UsageView {
  totalTokens: number;
  totalActiveMs: number;
  /** Agents in total, and how many ran at all (reported tokens or a known start). */
  agents: number;
  byMachine: { id: string; machine: string; tokens: number; activeMs: number; agents: number }[];
  byProject: UsageGroup[];
}

/** Time since the agent started, as this window saw it: to now while it runs, to its end once closed. Idle time is not known, so this is not "active" time. Unknown when the start was not seen. */
export function activeMs(a: Pick<AgentTileModel, 'startedMs' | 'endedMs' | 'closed'>, now: number): number | undefined {
  if (!a.startedMs) return undefined;
  const end = a.closed && a.endedMs ? a.endedMs : now;
  return Math.max(0, end - a.startedMs);
}

export function usageView(fleet: readonly AgentTileModel[], now: number): UsageView {
  const machines = new Map<string, { id: string; machine: string; tokens: number; activeMs: number; agents: number }>();
  const projects = new Map<string, UsageGroup>();
  let totalTokens = 0;
  let totalActiveMs = 0;
  for (const a of fleet) {
    const tokens = a.tokens ?? 0;
    const active = activeMs(a, now);
    const pct = contextPercent(a.session);
    totalTokens += tokens;
    totalActiveMs += active ?? 0;
    const m = machines.get(a.machineId ?? a.machine) ?? { id: a.machineId ?? a.machine, machine: a.machine, tokens: 0, activeMs: 0, agents: 0 };
    m.tokens += tokens; m.activeMs += active ?? 0; m.agents += 1;
    machines.set(a.machineId ?? a.machine, m);
    const pk = `${a.machineId ?? a.machine}\u0000${a.projectId ?? a.workspace}`;
    const g = projects.get(pk) ?? { key: pk, machine: a.machine, project: a.project || a.workspace, tokens: 0, activeMs: 0, agents: [] };
    g.tokens += tokens; g.activeMs += active ?? 0;
    g.agents.push({ id: a.id, name: a.name, accent: a.accent, kind: a.kind || a.name, tokens, contextPct: pct, activeMs: active, running: !a.closed });
    projects.set(pk, g);
  }
  const heaviest = <T extends { tokens: number }>(l: T[]) => l.sort((x, y) => y.tokens - x.tokens);
  return { totalTokens, totalActiveMs, agents: fleet.length, byMachine: heaviest([...machines.values()]), byProject: heaviest([...projects.values()]).map((g) => ({ ...g, agents: heaviest(g.agents) })) };
}

/** Whether there is anything to show: any tokens, a context level or a known running time. */
export const hasUsage = (fleet: readonly AgentTileModel[]): boolean => fleet.some((a) => (a.tokens ?? 0) > 0 || contextPercent(a.session) !== undefined || !!a.startedMs);
