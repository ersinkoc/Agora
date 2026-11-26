import { getLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { agentKindOf } from './fleet';
import { visible } from './hidden';

// ADR-0091: the plan limits of subscription tools, as the nodes read them from the tools' own files. Pure: the window's clock is a
// parameter, nothing here reads it. A limit is only ever what a tool reported: with no observation the answer is "unknown", never a
// number worked out from tokens or a profile. No money anywhere.

/** One observation, as the hub replays it (`AgentQuotaObserved`), keyed by machine, tool and window. */
export interface QuotaObs {
  machineId: string;
  tool: string;
  /** `five_hour`, `seven_day` or `minutes:<n>`. */
  window: string;
  usedPercent: number;
  resetsAtMs: number;
  observedAtMs: number;
}

/** From this percentage a window is shown as getting full (amber), and from the second one as almost used up. */
export const WARN_PERCENT = 80;
export const CRITICAL_PERCENT = 95;
const FALLBACK_WINDOW_MS = 5 * 3_600_000;

export type QuotaLevel = 'normal' | 'high' | 'critical';
export const levelOf = (percent: number): QuotaLevel => (percent >= CRITICAL_PERCENT ? 'critical' : percent >= WARN_PERCENT ? 'high' : 'normal');
const clampPercent = (p: number): number => Math.min(100, Math.max(0, Math.round(p)));

/**
 * The window's length in minutes from its label; undefined for a label that is not one of ours. The length comes from the label,
 * never from a tool's field name.
 */
export function windowMinutes(window: string): number | undefined {
  if (window === 'five_hour') return 300;
  if (window === 'seven_day') return 10080;
  const m = /^minutes:(\d{1,7})$/.exec(window);
  const n = m ? Number(m[1]) : 0;
  return n > 0 ? n : undefined;
}

/** "Five hours", "Seven days", "45 minutes": derived from the window's length. */
export function windowLabel(window: string): string {
  if (window === 'five_hour') return t('quota-window-five-hour');
  if (window === 'seven_day') return t('quota-window-seven-day');
  const minutes = windowMinutes(window);
  return minutes === undefined ? visible(window) : t('quota-window-minutes', { minutes });
}

/** The tool's name in the window: the agent kind with a capital ("codex" becomes "Codex"). */
export function toolName(tool: string): string {
  const shown = visible(tool);
  return shown ? shown.charAt(0).toUpperCase() + shown.slice(1) : shown;
}

// fresh: observed, its window still running, and not older than the window is long. stale: observed longer ago than the window is
// long, so the number may no longer be true. reset: the window's reset time has passed; the old percentage is not shown until a new
// observation arrives. unknown: nothing was ever observed.
export type QuotaState = 'fresh' | 'stale' | 'reset';

/**
 * The state of one observation at `now` (the window's own clock, milliseconds since the epoch).
 *
 * Clocks: `observedAtMs` and `resetsAtMs` are absolute times (the node's clock and the tool's), each compared here with the window's
 * clock; no duration is taken between two different machines' clocks beyond that. A machine whose clock is a little off makes an age
 * wrong by that much at most, and an age is never negative (an observation "from the future" counts as just now). The reset check has
 * no tolerance on purpose: a reset time that has passed on this clock is a reset.
 */
export function classify(o: Pick<QuotaObs, 'resetsAtMs' | 'observedAtMs' | 'window'>, now: number): { state: QuotaState; ageMs: number } {
  const ageMs = Math.max(0, now - o.observedAtMs);
  // No reset time, or one that has passed: a reset. A window is never fresh "forever" for lack of a reset time.
  if (o.resetsAtMs <= 0 || now >= o.resetsAtMs) return { state: 'reset', ageMs };
  const length = (windowMinutes(o.window) ?? FALLBACK_WINDOW_MS / 60_000) * 60_000;
  return { state: ageMs > length ? 'stale' : 'fresh', ageMs };
}

export interface QuotaBar {
  key: string;
  machineId: string;
  machine: string;
  tool: string;
  window: string;
  state: QuotaState;
  /** Absent when the state is reset or unknown: the old percentage is never shown then. */
  percent?: number;
  level: QuotaLevel;
  resetsAtMs: number;
  ageMs: number;
}
export interface QuotaMachine { machineId: string; machine: string; bars: QuotaBar[] }
export interface QuotaTool { tool: string; machines: QuotaMachine[] }

/** The machine an agent runs on, as the nodes name it in an observation. Never the display name (two machines can share one): an agent without an id is left out. */
export const machineKeyOf = (a: AgentTileModel): string | undefined => a.machineId ?? a.workspaceRef?.node;

/** The kinds known to have plan limits a tool can report. Any other kind (an API key, a shell, a tool with no plan) is shown nothing about. */
export const PLAN_TOOLS: readonly string[] = ['codex', 'claude-code', 'claude'];


export interface QuotaView {
  tools: QuotaTool[];
  /** Kinds known to have plan limits that have agents here and no reading from any machine: one quiet line, never a row each. */
  unknown: string[];
}

/**
 * What the Usage panel draws: per tool, per machine (a machine's own plan state; two machines on one account may disagree for a moment),
 * one bar per window shortest first; and the kinds with plan limits that reported nothing.
 */
export function quotaView(obs: readonly QuotaObs[], fleet: readonly AgentTileModel[], machineNames: ReadonlyMap<string, string>, now: number): QuotaView {
  const tools = new Map<string, Map<string, QuotaMachine>>();
  for (const o of obs) {
    const byMachine = tools.get(o.tool) ?? new Map<string, QuotaMachine>();
    tools.set(o.tool, byMachine);
    const m = byMachine.get(o.machineId) ?? { machineId: o.machineId, machine: machineNames.get(o.machineId) ?? o.machineId, bars: [] };
    byMachine.set(o.machineId, m);
    const { state, ageMs } = classify(o, now);
    const percent = clampPercent(o.usedPercent);
    m.bars.push({
      key: `${o.machineId}\u0000${o.tool}\u0000${o.window}`, machineId: o.machineId, machine: m.machine, tool: o.tool, window: o.window, state,
      ...(state === 'reset' ? {} : { percent }), level: state === 'reset' ? 'normal' : levelOf(percent), resetsAtMs: o.resetsAtMs, ageMs,
    });
  }
  const unknown = new Set<string>();
  for (const a of fleet) {
    const tool = agentKindOf(a);
    if (machineKeyOf(a) !== undefined && PLAN_TOOLS.includes(tool) && !tools.has(tool)) unknown.add(tool);
  }
  const length = (w: string) => windowMinutes(w) ?? Number.MAX_SAFE_INTEGER;
  return {
    unknown: [...unknown].sort(),
    tools: [...tools].sort(([x], [y]) => x.localeCompare(y)).map(([tool, byMachine]) => ({
      tool,
      machines: [...byMachine.values()].sort((x, y) => x.machine.localeCompare(y.machine)).map((m) => ({
        ...m, bars: [...m.bars].sort((x, y) => length(x.window) - length(y.window) || x.window.localeCompare(y.window)),
      })),
    })),
  };
}

interface Full { percent: number; level: QuotaLevel; window: string; resetsAtMs: number }

/** The fullest fresh window of a tool on a machine from `min` percent; a stale one may no longer be true, a reset one is not full any more. */
function fullest(tool: string, machineId: string, obs: readonly QuotaObs[], now: number, min: number): Full | undefined {
  let best: Full | undefined;
  for (const o of obs) {
    if (o.tool !== tool || o.machineId !== machineId || classify(o, now).state !== 'fresh') continue;
    const percent = clampPercent(o.usedPercent);
    if (percent >= min && (!best || percent > best.percent)) best = { percent, level: levelOf(percent), window: o.window, resetsAtMs: o.resetsAtMs };
  }
  return best;
}

/** The chip on an agent's card or row: only from `WARN_PERCENT`. */
export const chipFor = (a: AgentTileModel, obs: readonly QuotaObs[], now: number): Full | undefined => {
  const machine = machineKeyOf(a);
  return machine === undefined ? undefined : fullest(agentKindOf(a), machine, obs, now, WARN_PERCENT);
};

/** The start window's hint for a chosen kind on a chosen machine, from `CRITICAL_PERCENT`. It never blocks anything. */
export const startHint = (tool: string, machineId: string, obs: readonly QuotaObs[], now: number): Full | undefined => fullest(tool, machineId, obs, now, CRITICAL_PERCENT);

/** "1 h 20 min", "3 d 4 h" (Turkish: "1 sa 20 dk", "3 gün 4 sa"): the time until a reset, never seconds. The unit words are locale strings. */
export function untilText(ms: number): string {
  const min = Math.max(1, Math.ceil(ms / 60_000));
  if (min >= 2880) return t('quota-span-days', { days: Math.floor(min / 1440), hours: Math.floor((min % 1440) / 60) });
  if (min < 60) return t('quota-span-minutes', { minutes: min });
  return t('quota-span-hours', { hours: Math.floor(min / 60), minutes: min % 60 });
}

/** The reset time in the window's own timezone ("14:30 CET", with the day when it is not today). */
export function resetAtText(resetsAtMs: number, now: number): string {
  const sameDay = new Date(resetsAtMs).toDateString() === new Date(now).toDateString();
  try {
    return new Intl.DateTimeFormat(getLanguage(), { ...(sameDay ? {} : { weekday: 'short', day: 'numeric', month: 'short' }), hour: '2-digit', minute: '2-digit', timeZoneName: 'short' }).format(resetsAtMs);
  } catch { return new Date(resetsAtMs).toISOString(); }
}

/** "Resets in 1 h 20 min (14:30 CET)". */
export const resetsText = (resetsAtMs: number, now: number): string => t('quota-resets', { time: untilText(resetsAtMs - now), when: resetAtText(resetsAtMs, now) });
