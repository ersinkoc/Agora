import { describe, expect, it } from 'vitest';
import type { ChatLine } from './chat';
import { atBottom, grew, jumped, resized, scrolled, sizeOf, start } from './follow';
import { changedFiles, stepsSummary, transcript, windowed } from './transcript';

const at = (h: number, d = 5) => new Date(2026, 10, d, h, 0, 0).getTime();
const l = (id: number, kind: ChatLine['kind'], over: Partial<ChatLine> = {}): ChatLine => ({ id, atMs: at(9), role: 'agent', kind, text: '', path: '', ...over });
const tool = (id: number, name: string, args: object, atMs = at(9)) => l(id, 'tool', { text: JSON.stringify({ tool: name, arguments: args }), atMs });

describe('transcript', () => {
  it('joins the chunks of one reply, folds tool calls in a row and opens only the latest plan', () => {
    const items = transcript([
      l(1, 'text', { role: 'user', text: 'fix it' }), l(2, 'plan', { text: 'a\nb' }),
      tool(3, 'Read', { file_path: 'a.rs' }), tool(4, 'Bash', { command: 'cargo test' }),
      l(5, 'text', { text: 'Done, ' }), l(6, 'text', { text: 'it passes.' }), l(7, 'plan', { text: 'a\nb\nc' }),
    ]);
    expect(items.map((x) => (x.kind === 'steps' ? `steps:${x.lines.length}` : x.kind === 'reply' ? `reply:${x.text}` : x.kind === 'plan' ? `plan${x.latest ? '*' : ''}` : x.kind)))
      .toEqual(['date', 'user', 'plan', 'steps:2', 'reply:Done, it passes.', 'plan*']);
  });
  it('is empty for no lines and gives entries without a time no date', () => {
    expect(transcript([])).toEqual([]);
    expect(transcript([l(1, 'text', { atMs: 0 })]).map((x) => x.kind)).toEqual(['reply']);
  });
  it('puts a separator where the day changes, and not otherwise', () => {
    const items = transcript([l(1, 'text', { atMs: at(9, 5) }), l(2, 'text', { role: 'user', atMs: at(23, 5) }), l(3, 'text', { atMs: at(1, 6) })]);
    expect(items.filter((x) => x.kind === 'date')).toHaveLength(2);
  });
  it('does not mix a user message into a reply, and keeps notices apart', () => {
    const items = transcript([l(1, 'text', { text: 'a' }), l(2, 'text', { role: 'user', text: 'q' }), l(3, 'text', { text: 'b' }), l(4, 'notice', { code: 'adapter.turn_failed' })]);
    expect(items.map((x) => x.kind)).toEqual(['date', 'reply', 'user', 'reply', 'notice']);
  });
});

describe('steps summary and changed files', () => {
  it('counts what a group did and names the last step', () => {
    const s = stepsSummary([tool(1, 'Read', { file_path: 'a.rs' }, at(9)), tool(2, 'Read', { file_path: 'b.rs' }, at(9)), tool(3, 'Bash', { command: 'ls' }, at(9) + 5000)]);
    expect(s.total).toBe(3);
    expect(s.actions[0]).toEqual({ action: 'read', count: 2 });
    expect(s.spanMs).toBe(5000);
    expect(s.last).not.toBe('');
  });
  it('adds a file changed twice once', () => {
    const d = (id: number, text: string) => l(id, 'diff', { path: 'src/a.rs', text });
    const files = changedFiles([d(1, '@@ -1 +1 @@\n-a\n+b'), d(2, '@@ -1 +1,2 @@\n-b\n+c\n+d'), l(3, 'diff', { path: 'x.md', text: '+z' })]);
    expect(files.map((f) => [f.path, f.entries])).toEqual([['src/a.rs', 2], ['x.md', 1]]);
    expect(files[0]?.added).toBe(3);
    expect(files[0]?.removed).toBe(2);
  });
});

describe('bounded history', () => {
  it('shows the newest and counts what is behind "show earlier"', () => {
    const all = Array.from({ length: 5000 }, (_, i) => i);
    const w = windowed(all, 200);
    expect(w.items.length).toBe(200);
    expect(w.items[199]).toBe(4999);
    expect(w.hidden).toBe(4800);
    expect(windowed([1, 2], 200)).toEqual({ items: [1, 2], hidden: 0 });
  });
});

describe('follow the tail', () => {
  it('knows the bottom with a little slack', () => {
    expect(atBottom({ scrollTop: 900, clientHeight: 100, scrollHeight: 1000 })).toBe(true);
    expect(atBottom({ scrollTop: 860, clientHeight: 100, scrollHeight: 1000 })).toBe(true); // 40 px up
    expect(atBottom({ scrollTop: 500, clientHeight: 100, scrollHeight: 1000 })).toBe(false);
  });
  it('follows while at the bottom and counts what arrives once the reader scrolled up', () => {
    let f = start;
    f = grew(f, 3);
    expect(f).toEqual({ following: true, unseen: 0 });
    f = scrolled(f, false);
    f = grew(grew(f, 2), 1);
    expect(f).toEqual({ following: false, unseen: 3 });
    expect(scrolled(f, true)).toEqual({ following: true, unseen: 0 }); // back at the bottom by hand
    expect(jumped()).toEqual({ following: true, unseen: 0 }); // or by the pill
    expect(grew(f, 0)).toBe(f);
  });
  it('tells a scroll that came with a size change from the reader scrolling (#1060)', () => {
    const seen = sizeOf({ clientHeight: 500, scrollHeight: 2000 });
    expect(resized(seen, { clientHeight: 500, scrollHeight: 2000 })).toBe(false); // the same sizes: the reader scrolled
    expect(resized(seen, { clientHeight: 300, scrollHeight: 2000 })).toBe(true); // the log was resized
    expect(resized(seen, { clientHeight: 500, scrollHeight: 2400 })).toBe(true); // its content was
    expect(resized(undefined, { clientHeight: 500, scrollHeight: 2000 })).toBe(false); // nothing seen yet
    expect(resized({ clientHeight: 0, scrollHeight: 0 }, { clientHeight: 500, scrollHeight: 2000 })).toBe(false); // seen before it was drawn
  });
});
