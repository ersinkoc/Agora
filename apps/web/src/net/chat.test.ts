import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { chatLine, mergeLines, type ChatLine } from './chat';

const line = (id: number): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'text', text: `m${id}`, path: '' });

describe('chat lines', () => {
  it('reads an entry', () => {
    const l = chatLine(agora.v1.ChatEntry.create({ entryId: 3, atMs: 9, role: agora.v1.ChatRole.CHAT_ROLE_USER, kind: agora.v1.ChatKind.CHAT_KIND_DIFF, text: '@@', path: 'a.rs' }));
    expect(l).toEqual({ id: 3, atMs: 9, role: 'user', kind: 'diff', text: '@@', path: 'a.rs' });
  });
  it('merges without repeating a line that is already held', () => {
    expect(mergeLines([line(1), line(2)], [line(2), line(3)]).map((l) => l.id)).toEqual([1, 2, 3]);
    expect(mergeLines([], [line(1)]).map((l) => l.id)).toEqual([1]);
  });
});
