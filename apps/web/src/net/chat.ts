// What an agent without a terminal says, as the node streams it (`chat.subscribe`, ADR-0038). The text was written by a model or
// a person: it is shown as plain text, never as HTML.
import { agora } from '../gen/agora.js';

// ADR-0090: a tool call and a plan are ONE entry that the node sends again, with the same id, each time it changes; the fields below are
// set only when the node sent them (an old node sends none, and the screen is then what it was).
export type CallStatus = 'pending' | 'in_progress' | 'completed' | 'failed';
export type StepState = 'pending' | 'in_progress' | 'completed';
export type CallKind = 'read' | 'edit' | 'delete' | 'move' | 'search' | 'run' | 'think' | 'fetch' | 'other';
export type Outcome = 'pending' | 'approved' | 'denied' | 'cancelled';
export interface PlanStepLine { text: string; status?: StepState; priority?: 'high' | 'medium' | 'low' }

// A notice (ADR-0058) has no text: `code` and `params` say what happened and the window says it in the reader's language.
export interface ChatLine {
  id: number; atMs: number; role: 'agent' | 'user'; kind: 'text' | 'tool' | 'plan' | 'diff' | 'notice' | 'thought' | 'permission'; text: string; path: string; code?: string; params?: Record<string, string>;
  callId?: string; callKind?: CallKind; status?: CallStatus; locations?: { path: string; line: number }[]; output?: string; outputCut?: boolean; startedMs?: number; endedMs?: number;
  steps?: PlanStepLine[]; queued?: boolean; approvalId?: string; outcome?: Outcome;
}
export interface ChatSource { connect(onLines: (lines: ChatLine[], replace: boolean) => void): { close(): void } }

const KINDS: readonly ChatLine['kind'][] = ['text', 'text', 'tool', 'plan', 'diff', 'notice', 'thought', 'permission'];
// `ToolKind` (1..10, ACP's names), `ToolStatus` (1..4), `PlanPriority` and `PermissionOutcome`: 0 and anything a newer node adds read as "not said".
const CALL_KINDS: readonly (CallKind | undefined)[] = [undefined, 'read', 'edit', 'delete', 'move', 'search', 'run', 'think', 'fetch', 'other', 'other'];
const STATUSES: readonly (CallStatus | undefined)[] = [undefined, 'pending', 'in_progress', 'completed', 'failed'];
const PRIORITIES: readonly (PlanStepLine['priority'])[] = [undefined, 'high', 'medium', 'low'];
const OUTCOMES: readonly (Outcome | undefined)[] = [undefined, 'pending', 'approved', 'denied', 'cancelled'];
const ms = (n: { toString(): string } | number | null | undefined): number => Number(n?.toString() ?? 0) || 0;

export function chatLine(e: agora.v1.ChatEntry.$Properties): ChatLine {
  const line: ChatLine = {
    id: Number(e.entryId?.toString() ?? 0),
    atMs: Number(e.atMs?.toString() ?? 0),
    role: e.role === agora.v1.ChatRole.CHAT_ROLE_USER ? 'user' : 'agent',
    kind: KINDS[e.kind ?? 0] ?? 'text',
    text: e.text ?? '',
    path: e.path ?? '',
  };
  if (line.kind === 'notice') {
    line.code = e.code ?? '';
    line.params = { ...(e.params ?? {}) };
  }
  // ADR-0090. Only what is there is set, so an entry from an old node reads exactly as before.
  if (e.callId) line.callId = e.callId;
  const ck = CALL_KINDS[e.toolKind ?? 0];
  if (ck) line.callKind = ck;
  const st = STATUSES[e.status ?? 0];
  if (st) line.status = st;
  if (e.paths?.length) {
    const places = e.paths.map((p) => ({ path: p.path ?? '', line: p.line ?? 0 })).filter((p) => p.path);
    if (places.length) line.locations = places;
  }
  if (e.output) line.output = e.output;
  if (e.outputTruncated) line.outputCut = true;
  if (ms(e.startedMs)) line.startedMs = ms(e.startedMs);
  if (ms(e.endedMs)) line.endedMs = ms(e.endedMs);
  if (e.planSteps?.length) {
    line.steps = e.planSteps.map((p): PlanStepLine => {
      const s = STATUSES[p.status ?? 0];
      const pr = PRIORITIES[p.priority ?? 0];
      return { text: p.text ?? '', ...(s && s !== 'failed' ? { status: s } : {}), ...(pr ? { priority: pr } : {}) };
    });
  }
  if (e.queued) line.queued = true;
  if (e.approvalId) line.approvalId = e.approvalId;
  const oc = OUTCOMES[e.permissionOutcome ?? 0];
  if (oc) line.outcome = oc;
  return line;
}

// The locale id of a notice: `adapter.turn_failed` → `notice-adapter-turn_failed`. Codes are dotted lowercase words (the node drops
// anything else), so the id is always a valid Fluent id.
export const noticeKey = (code: string): string => `notice-${code.replace(/\./g, '-')}`;

// What a notice says, in words: its own message when the window knows the code, a general one naming the code otherwise. Parameters
// are passed as plain strings (a stderr tail is the agent's text, shown as text). `exit_code` and `stderr_tail` are optional on the wire.
export function noticeText(line: ChatLine, tr: (id: string, args?: Record<string, string>) => string): string {
  const code = line.code ?? '';
  const args = { exit_code: '?', stderr_tail: '', detail: '', rpc_code: '', ...(line.params ?? {}) };
  const id = noticeKey(code);
  const said = tr(id, args);
  return said === id ? tr('notice-other', { code }) : said;
}

// The fields that change in place; a repeat that differs in none of them says nothing new.
const SAME_KEYS: readonly (keyof ChatLine)[] = ['role', 'text', 'kind', 'path', 'code', 'approvalId', 'callId', 'status', 'callKind', 'output', 'outputCut', 'startedMs', 'endedMs', 'queued', 'outcome', 'atMs'];
function same(a: ChatLine, b: ChatLine): boolean {
  return SAME_KEYS.every((k) => a[k] === b[k]) && JSON.stringify(a.steps) === JSON.stringify(b.steps) && JSON.stringify(a.locations) === JSON.stringify(b.locations) && JSON.stringify(a.params) === JSON.stringify(b.params);
}

/** The most entries a window holds per conversation. The node keeps 500; a hostile or buggy stream of new ids cannot make this grow. */
export const MAX_LINES = 5000;

// `held` is sorted by id (appended in order, inserted in place), so an entry is found by binary search: no per-chunk index to rebuild.
function find(list: readonly ChatLine[], id: number): { at: number; found: boolean } {
  let lo = 0, hi = list.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if ((list[mid] as ChatLine).id < id) lo = mid + 1; else hi = mid;
  }
  return { at: lo, found: lo < list.length && (list[lo] as ChatLine).id === id };
}

/** The newest `cap` entries, and how many older ones were left out (the window says "earlier entries are not loaded"). */
export function trimLines(lines: ChatLine[], cap = MAX_LINES): { lines: ChatLine[]; dropped: number } {
  return lines.length > cap ? { lines: lines.slice(lines.length - cap), dropped: lines.length - cap } : { lines, dropped: 0 };
}

// Adds new lines after the ones already held. An entry the node sends again with the same id (a tool call or a plan that changed, a
// queued prompt that was sent) REPLACES the held one in its place (ADR-0090): the transcript has one row for it, never two, and nothing
// moves but that row. A repeat that changes nothing keeps the held object, so nothing redraws. An id that is not held is NEVER dropped: a
// newer one is appended, an older one (ids that restarted after the node did, a late frame) goes in its place by id. A snapshot replaces
// the whole held set instead (the caller). Each entry costs a binary search, not a rebuilt index.
export function mergeLines(held: readonly ChatLine[], incoming: readonly ChatLine[]): ChatLine[] {
  let out: ChatLine[] | undefined;
  for (const l of incoming) {
    const cur = out ?? held;
    const last = cur.length ? (cur[cur.length - 1] as ChatLine).id : 0;
    if (l.id > last) { out ??= [...held]; out.push(l); continue; }
    const { at, found } = find(cur, l.id);
    if (found) {
      if (same(cur[at] as ChatLine, l)) continue;
      out ??= [...held];
      out[at] = l;
    } else {
      out ??= [...held];
      out.splice(at, 0, l);
    }
  }
  return out ?? (held as ChatLine[]);
}

// ADR-0090: prompts waiting behind a turn. A queued user entry waits until the node sends the same entry again without the flag, or
// says it will not send it: the notice `adapter.queue_dropped` is NEWER than every entry it drops (ids grow), and the schema has no way
// to remove an entry, so the entry stays and is marked "not sent" instead of waiting for ever.
export const QUEUE_DROPPED = 'adapter.queue_dropped';
export function queueState(lines: readonly ChatLine[]): { waiting: ChatLine[]; notSent: Set<number> } {
  let dropped = 0;
  for (const l of lines) if (l.kind === 'notice' && l.code === QUEUE_DROPPED) dropped = l.id;
  const waiting: ChatLine[] = [];
  const notSent = new Set<number>();
  for (const l of lines) {
    if (l.role !== 'user' || !l.queued) continue;
    if (l.id < dropped) notSent.add(l.id); else waiting.push(l);
  }
  return { waiting, notSent };
}

// ADR-0090: whether anything on this conversation shows that its node speaks the rich form (capability `chat.rich`). The window cannot
// read a node's capabilities (the snapshot carries none), so it goes by what the node sent: a call with an id or a status, a thought, a
// plan with steps, a permission record, a queued prompt.
export const isRich = (lines: readonly ChatLine[]): boolean => lines.some((l) => l.callId !== undefined || l.status !== undefined || l.kind === 'thought' || l.kind === 'permission' || l.steps !== undefined || l.queued === true);
