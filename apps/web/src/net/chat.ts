// What an agent without a terminal says, as the node streams it (`chat.subscribe`, ADR-0038). The text was written by a model or
// a person: it is shown as plain text, never as HTML.
import { agora } from '../gen/agora.js';

export interface ChatLine { id: number; atMs: number; role: 'agent' | 'user'; kind: 'text' | 'tool' | 'plan' | 'diff'; text: string; path: string }
export interface ChatSource { connect(onLines: (lines: ChatLine[], replace: boolean) => void): { close(): void } }

const KINDS = ['text', 'text', 'tool', 'plan', 'diff'] as const;

export function chatLine(e: agora.v1.ChatEntry.$Properties): ChatLine {
  return {
    id: Number(e.entryId?.toString() ?? 0),
    atMs: Number(e.atMs?.toString() ?? 0),
    role: e.role === agora.v1.ChatRole.CHAT_ROLE_USER ? 'user' : 'agent',
    kind: KINDS[e.kind ?? 0] ?? 'text',
    text: e.text ?? '',
    path: e.path ?? '',
  };
}

// Adds new lines after the ones already held, skipping any whose id is already there (a snapshot may overlap what arrived).
export function mergeLines(held: readonly ChatLine[], incoming: readonly ChatLine[]): ChatLine[] {
  const last = held.length ? (held[held.length - 1] as ChatLine).id : 0;
  return [...held, ...incoming.filter((l) => l.id > last)];
}
