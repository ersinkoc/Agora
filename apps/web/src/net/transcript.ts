import type { ChatLine } from './chat';
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
  | { kind: 'steps'; id: string; lines: ChatLine[] }
  | { kind: 'plan'; id: string; line: ChatLine; latest: boolean }
  | { kind: 'diff'; id: string; line: ChatLine }
  | { kind: 'notice'; id: string; line: ChatLine };

const day = (ms: number): string => { const d = new Date(ms); return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`; };

export function transcript(lines: readonly ChatLine[]): TranscriptItem[] {
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
    counts.set(v.action, (counts.get(v.action) ?? 0) + 1);
    last = ((v.title || v.subject || v.raw).split('\n')[0] ?? '').replace(/`/g, '');
  }
  const first = lines.find((l) => l.atMs > 0)?.atMs;
  const end = [...lines].reverse().find((l) => l.atMs > 0)?.atMs;
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

/** The newest `shown` items of a long history, and how many older ones wait behind "show earlier". */
export function windowed<T>(items: readonly T[], shown: number): { items: T[]; hidden: number } {
  const hidden = Math.max(0, items.length - shown);
  return { items: hidden ? items.slice(hidden) : [...items], hidden };
}

/** A folded group must never hide that the agent needs you or that it failed: the newest group says so (and opens) while it is true.
 *  The data has no per-call status, so this is the agent's own state, not a guess about one call. */
export type GroupFlag = 'needs-you' | 'failed';
export function groupFlag(agent: { state: string; closed?: boolean; exitCode?: number; interrupted?: unknown }, newest: boolean): GroupFlag | undefined {
  if (!newest) return undefined;
  if (!agent.closed) return agent.state === 'needs-you' ? 'needs-you' : undefined;
  return endedWithError(agent) ? 'failed' : undefined;
}
