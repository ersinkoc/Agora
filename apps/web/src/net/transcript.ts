import { notSentIds, type ChatLine } from './chat';
import { parseHunks } from './diff';
import { endedWithError } from '../model';
import { toolView, type ToolAction } from './tool';

// How a conversation reads: the node's entries (one per chunk of text, per tool call, per plan, per diff) become what a person sees.
// Consecutive chunks of what the agent said are one reply, consecutive tool calls are one folded group, and a new day gets a
// separator. Pure, so it is tested without a page; the entries are untrusted text and stay text.

export type TranscriptItem =
  | { kind: 'date'; id: string; atMs: number }
  | { kind: 'user'; id: string; line: ChatLine }
  | { kind: 'reply'; id: string; text: string; atMs: number }
  // ADR-0090: what the agent thought (chunks joined like a reply); `endMs` is when the next thing happened, so "Thought for N s".
  | { kind: 'thought'; id: string; text: string; atMs: number; endMs: number }
  | { kind: 'permission'; id: string; line: ChatLine }
  | { kind: 'steps'; id: string; lines: ChatLine[] }
  | { kind: 'plan'; id: string; line: ChatLine; latest: boolean }
  | { kind: 'diff'; id: string; line: ChatLine }
  | { kind: 'notice'; id: string; line: ChatLine };

// #908 (RM-44): the order a person reads. The node numbers entries as they arrive, so a prompt that waited behind a turn has a smaller
// id than the answers of the turn before it; when it is sent its entry comes again with the time it went (`at_ms` is when an entry's
// event happened). So a prompt of the person's is placed after the agent's entry it was seen to follow (`sentAfter`, this window saw it
// sent), else by time: before the first other entry that is not older, ties and out-of-order times broken by id. An entry without a
// time (0, an old node) moves nothing past it. A prompt still waiting is drawn after everything. The agent's own entries keep the node's
// order. Pure; the held lines are not changed.
export function ordered(lines: readonly ChatLine[]): ChatLine[] {
  const notSent = notSentIds(lines);
  const others = lines.filter((l) => l.role !== 'user');
  const firstAfter = (id: number): number => { // the index of the first other entry with a larger id
    let lo = 0, hi = others.length;
    while (lo < hi) { const mid = (lo + hi) >>> 1; if ((others[mid] as ChatLine).id <= id) lo = mid + 1; else hi = mid; }
    return lo;
  };
  const before: ChatLine[][] = Array.from({ length: others.length + 1 }, () => []);
  const waiting: ChatLine[] = [];
  for (const l of lines) {
    if (l.role !== 'user') continue;
    if (l.queued && !notSent.has(l.id)) { waiting.push(l); continue; }
    let i: number;
    if (l.sentAfter !== undefined) i = firstAfter(Math.max(l.sentAfter, l.id));
    else {
      i = firstAfter(l.id);
      while (i < others.length && (others[i] as ChatLine).atMs > 0 && (others[i] as ChatLine).atMs < l.atMs) i++;
    }
    (before[i] as ChatLine[]).push(l);
  }
  const out: ChatLine[] = [];
  others.forEach((o, i) => { const b = before[i] as ChatLine[]; if (b.length) out.push(...b.sort((x, y) => x.id - y.id)); out.push(o); });
  out.push(...(before[others.length] as ChatLine[]).sort((x, y) => x.id - y.id), ...waiting);
  return out;
}

const day = (ms: number): string => { const d = new Date(ms); return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`; };

export function transcript(held: readonly ChatLine[]): TranscriptItem[] {
  const lines = ordered(held);
  const out: TranscriptItem[] = [];
  let lastPlan = -1;
  lines.forEach((l, i) => { if (l.role === 'agent' && l.kind === 'plan') lastPlan = i; });
  let lastDay = '';
  lines.forEach((l, i) => {
    // A separator before the first entry with a time, and wherever the day changes (entries without a time never start one).
    if (l.atMs > 0 && day(l.atMs) !== lastDay) { lastDay = day(l.atMs); out.push({ kind: 'date', id: `d${l.id}`, atMs: l.atMs }); }
    const prev = out[out.length - 1];
    if (l.role === 'user') { out.push({ kind: 'user', id: `u${l.id}`, line: l }); return; }
    switch (l.kind) {
      case 'tool':
        if (prev?.kind === 'steps') prev.lines.push(l); else out.push({ kind: 'steps', id: `s${l.id}`, lines: [l] });
        return;
      case 'plan': out.push({ kind: 'plan', id: `p${l.id}`, line: l, latest: i === lastPlan }); return;
      case 'diff': out.push({ kind: 'diff', id: `f${l.id}`, line: l }); return;
      case 'notice': out.push({ kind: 'notice', id: `n${l.id}`, line: l }); return;
      case 'permission': out.push({ kind: 'permission', id: `a${l.id}`, line: l }); return;
      case 'thought': {
        const next = lines[i + 1];
        const endMs = next && next.kind !== 'thought' && next.atMs > 0 ? next.atMs : l.atMs;
        if (prev?.kind === 'thought') { prev.text += l.text; prev.endMs = Math.max(prev.endMs, endMs); } else out.push({ kind: 'thought', id: `h${l.id}`, text: l.text, atMs: l.atMs, endMs: Math.max(l.atMs, endMs) });
        return;
      }
      default:
        // The adapter sends what the agent says in chunks; they are one message, so the reply grows in place instead of stacking.
        if (prev?.kind === 'reply') prev.text += l.text; else out.push({ kind: 'reply', id: `r${l.id}`, text: l.text, atMs: l.atMs });
    }
  });
  return out;
}

export interface StepsSummary {
  /** What the group did, by kind, in the order each first happened. */
  actions: { action: ToolAction; count: number }[];
  total: number;
  /** The last step's title or subject, so a group still running says what it is on. */
  last: string;
  /** From the first to the last step's time, when the entries carry one; undefined under one second. */
  spanMs?: number;
}

export function stepsSummary(lines: readonly ChatLine[]): StepsSummary {
  const counts = new Map<ToolAction, number>();
  let last = '';
  for (const l of lines) {
    const v = toolView(l.text);
    const action = actionOf(l, v.action);
    counts.set(action, (counts.get(action) ?? 0) + 1);
    last = ((v.title || v.subject || v.raw).split('\n')[0] ?? '').replace(/`/g, '');
  }
  // The node's own clock when it measured the calls (ADR-0090): from the first call's start to the last end; else the entries' times.
  const measured = lines.filter((l) => l.startedMs !== undefined);
  const first = measured.length ? Math.min(...measured.map((l) => l.startedMs as number)) : lines.find((l) => l.atMs > 0)?.atMs;
  const end = measured.length && measured.every((l) => l.endedMs !== undefined) ? Math.max(...measured.map((l) => l.endedMs as number)) : measured.length ? undefined : [...lines].reverse().find((l) => l.atMs > 0)?.atMs;
  const span = first !== undefined && end !== undefined ? end - first : 0;
  return { actions: [...counts].map(([action, count]) => ({ action, count })), total: lines.length, last, ...(span >= 1000 ? { spanMs: span } : {}) };
}

export interface ChangedFile { path: string; added: number; removed: number; entries: number }

/** The files the conversation changed, each once, with the lines added and removed across all its diffs, in the order first changed. */
export function changedFiles(lines: readonly ChatLine[]): ChangedFile[] {
  const byPath = new Map<string, ChangedFile>();
  for (const l of lines) {
    if (l.role !== 'agent' || l.kind !== 'diff' || !l.path) continue;
    const d = parseHunks(l.text);
    const f = byPath.get(l.path) ?? { path: l.path, added: 0, removed: 0, entries: 0 };
    f.added += d.added; f.removed += d.removed; f.entries += 1;
    byPath.set(l.path, f);
  }
  return [...byPath.values()];
}

/** #908: the groups of calls whose turn is over, by item id: every group followed by a prompt of the person's that was sent (a later
 *  turn began), and every group when the agent is not in a turn (`live` false: idle, finished, closed). A call of such a group that
 *  still says it runs or waits will not finish (the turn was stopped, or the agent moved on), so it is drawn as stopped, never spinning.
 *  ACP leaves marking them to the client. Pure. */
export function endedGroups(items: readonly TranscriptItem[], live: boolean): Set<string> {
  const ended = new Set<string>();
  let later = !live;
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i] as TranscriptItem;
    if (item.kind === 'user' && !item.line.queued) later = true;
    else if (item.kind === 'steps' && later) ended.add(item.id);
  }
  return ended;
}

/** The newest `shown` items of a long history, and how many older ones wait behind "show earlier". */
export function windowed<T>(items: readonly T[], shown: number): { items: T[]; hidden: number } {
  const hidden = Math.max(0, items.length - shown);
  return { items: hidden ? items.slice(hidden) : [...items], hidden };
}

/** A folded group must never hide that the agent needs you or that it failed: the newest group says so (and opens) while it is true
 *  (the agent's own state, not a guess about one call); `flagOf` adds the per-call status of ADR-0090 for every group. */
export type GroupFlag = 'needs-you' | 'failed' | 'call-failed';
type AgentFlagState = { state: string; closed?: boolean; exitCode?: number; interrupted?: unknown };
export function groupFlag(agent: AgentFlagState, newest: boolean): GroupFlag | undefined {
  if (!newest) return undefined;
  if (!agent.closed) return agent.state === 'needs-you' ? 'needs-you' : undefined;
  return endedWithError(agent) ? 'failed' : undefined;
}

/** What a call is, for its icon and its words: the kind the node sent (ACP's own, ADR-0090) when it says more than "other", else what the arguments suggest. */
export const actionOf = (line: ChatLine, inferred: ToolAction): ToolAction => (line.callKind && line.callKind !== 'other' ? line.callKind : inferred);

/** How many calls of a group failed. A group with one is never folded shut, wherever it sits in the conversation. */
export const failedCalls = (lines: readonly ChatLine[]): number => lines.filter((l) => l.status === 'failed').length;
/** The group right before a request that still waits for the user's answer is the one it is about: it opens too. */
export const asksBefore = (next: TranscriptItem | undefined): boolean => next?.kind === 'permission' && next.line.outcome === 'pending';

/** The flag a group of calls carries: the agent's own state on the newest one, and for ANY group a failed call, or a request right after
 *  it that waits for the user. Pure: the one rule the screen draws from. */
export function flagOf(agent: AgentFlagState, newest: boolean, lines: readonly ChatLine[], next?: TranscriptItem): GroupFlag | undefined {
  return groupFlag(agent, newest) ?? (asksBefore(next) ? 'needs-you' : failedCalls(lines) > 0 ? 'call-failed' : undefined);
}
