// What an agent without a terminal says, as the node streams it (`chat.subscribe`, ADR-0038). The text was written by a model or
// a person: it is shown as plain text, never as HTML.
import { agora } from '../gen/agora.js';

// A notice (ADR-0058) has no text: `code` and `params` say what happened and the window says it in the reader's language.
export interface ChatLine { id: number; atMs: number; role: 'agent' | 'user'; kind: 'text' | 'tool' | 'plan' | 'diff' | 'notice'; text: string; path: string; code?: string; params?: Record<string, string> }
export interface ChatSource { connect(onLines: (lines: ChatLine[], replace: boolean) => void): { close(): void } }

const KINDS: readonly ChatLine['kind'][] = ['text', 'text', 'tool', 'plan', 'diff', 'notice'];

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
  return line;
}

// The locale id of a notice: `adapter.turn_failed` → `notice-adapter-turn_failed`. Codes are dotted lowercase words (the node drops
// anything else), so the id is always a valid Fluent id.
export const noticeKey = (code: string): string => `notice-${code.replace(/\./g, '-')}`;

// What a notice says, in words: its own message when the window knows the code, a general one naming the code otherwise. Parameters
// are passed as plain strings (a stderr tail is the agent's text, shown as text). `exit_code` is optional on the wire.
export function noticeText(line: ChatLine, tr: (id: string, args?: Record<string, string>) => string): string {
  const code = line.code ?? '';
  const args = { exit_code: '?', ...(line.params ?? {}) };
  const id = noticeKey(code);
  const said = tr(id, args);
  return said === id ? tr('notice-other', { code }) : said;
}

// Adds new lines after the ones already held, skipping any whose id is already there (a snapshot may overlap what arrived).
export function mergeLines(held: readonly ChatLine[], incoming: readonly ChatLine[]): ChatLine[] {
  const last = held.length ? (held[held.length - 1] as ChatLine).id : 0;
  return [...held, ...incoming.filter((l) => l.id > last)];
}
