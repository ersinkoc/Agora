import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { chatLine, mergeLines, noticeText, type ChatLine } from './chat';
import { setLanguage, t } from '../i18n';

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
  it('reads a notice and says it in words, or names an unknown code', () => {
    setLanguage('en');
    const l = chatLine(agora.v1.ChatEntry.create({ entryId: 4, kind: agora.v1.ChatKind.CHAT_KIND_NOTICE, code: 'adapter.turn_stopped', params: { stop_reason: 'refusal' } }));
    expect([l.kind, l.code, l.params]).toEqual(['notice', 'adapter.turn_stopped', { stop_reason: 'refusal' }]);
    expect(noticeText(l, t)).toContain('refusal');
    const exited = chatLine(agora.v1.ChatEntry.create({ kind: agora.v1.ChatKind.CHAT_KIND_NOTICE, code: 'adapter.agent_exited', params: { stage: 'turn', stderr_tail: '<b>x</b>' } }));
    expect(noticeText(exited, t)).toContain('exit code ?');
    expect(noticeText(exited, t)).toContain('<b>x</b>'); // shown as text by React, never as HTML
    const empty = chatLine(agora.v1.ChatEntry.create({ kind: agora.v1.ChatKind.CHAT_KIND_NOTICE, code: 'adapter.turn_empty', params: { stderr_tail: 'invalid x-api-key (auth)' } }));
    expect(noticeText(empty, t)).toContain('invalid x-api-key (auth)');
    expect(noticeText(chatLine(agora.v1.ChatEntry.create({ kind: agora.v1.ChatKind.CHAT_KIND_NOTICE, code: 'adapter.turn_empty' })), t)).not.toContain('{');
    const unknown = chatLine(agora.v1.ChatEntry.create({ kind: agora.v1.ChatKind.CHAT_KIND_NOTICE, code: 'future.thing' }));
    expect(noticeText(unknown, t)).toContain('future.thing');
  });
});
