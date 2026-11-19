import { describe, expect, it } from 'vitest';
import type { ChatLine } from './chat';
import { atOf, groupTurns, initial, pairDiffs, planProgress, stepTone } from './chatView';
import { transcript } from './transcript';
import { preciseDuration } from './time';

const line = (id: number, kind: ChatLine['kind'], role: ChatLine['role'] = 'agent', atMs = 0): ChatLine => ({ id, atMs, role, kind, text: kind === 'tool' ? '{"tool":"execute","arguments":{}}' : 'x', path: '' });

describe('turns', () => {
  it('puts an agent\'s replies, steps, plan, diff and notice in one turn, and ends it at a person', () => {
    const lines = [line(1, 'text', 'user', 1000), line(2, 'text'), line(3, 'tool'), line(4, 'plan'), line(5, 'diff'), line(6, 'notice'), line(7, 'text', 'user'), line(8, 'text')];
    const rows = groupTurns(transcript(lines));
    expect(rows.map((r) => r.kind)).toEqual(['date', 'user', 'turn', 'user', 'turn']);
    const first = rows[2];
    expect(first?.kind === 'turn' && first.items.map((i) => i.kind)).toEqual(['reply', 'steps', 'plan', 'diff', 'notice']);
  });
  it('ends a turn at a new day', () => {
    const day = 86_400_000;
    const rows = groupTurns(transcript([line(1, 'text', 'agent', 1_700_000_000_000), line(2, 'text', 'agent', 1_700_000_000_000 + 2 * day)]));
    expect(rows.map((r) => r.kind)).toEqual(['date', 'turn', 'date', 'turn']);
  });
  it('dates the turn by its first entry that has a time', () => {
    const turn = groupTurns(transcript([line(1, 'text', 'agent', 5000), line(2, 'tool', 'agent', 6000)])).find((r) => r.kind === 'turn');
    expect(turn?.kind === 'turn' && turn.atMs).toBe(5000);
    expect(atOf({ kind: 'reply', id: 'r', text: '', atMs: 7 })).toBe(7);
    expect(atOf({ kind: 'steps', id: 's', lines: [line(1, 'tool'), line(2, 'tool', 'agent', 9)] })).toBe(9);
    expect(groupTurns([{ kind: 'steps', id: 's', lines: [line(1, 'tool')] }, { kind: 'reply', id: 'r', text: '', atMs: 4 }]).map((r) => r.kind === 'turn' && r.atMs)).toEqual([4]);
  });
});

describe('step tone', () => {
  it('tints edits and commands and leaves the rest quiet', () => {
    expect(stepTone('edit')).toBe('edit');
    expect(stepTone('run')).toBe('run');
    expect(stepTone('read')).toBe('plain');
    expect(stepTone('search')).toBe('plain');
    expect(stepTone('other')).toBe('plain');
  });
});

describe('plan progress', () => {
  it('is absent while no step has a status (today\'s data), never zero of N', () => {
    expect(planProgress([undefined, undefined])).toBeUndefined();
    expect(planProgress([])).toBeUndefined();
  });
  it('counts completed steps once statuses exist', () => {
    expect(planProgress(['completed', 'completed', 'in_progress', 'pending'])).toEqual({ done: 2, total: 4, percent: 50 });
  });
});

describe('an edit and its diff', () => {
  const edit = (id: number, path: string): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'tool', text: JSON.stringify({ tool: 'Edit', arguments: { file_path: path, old_string: 'a', new_string: 'b' } }), path: '' });
  const diff = (id: number, path: string): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'diff', text: '@@ -1 +1 @@\n-a\n+b\n', path });
  const turn = (lines: ChatLine[]) => groupTurns(transcript(lines)).flatMap((r) => (r.kind === 'turn' ? r.items : []));
  it('opens the diff inside the step of the same file, even when the step names an absolute path', () => {
    const { items, attached } = pairDiffs(turn([edit(1, '/home/a/src/x.rs'), edit(2, 'src/y.rs'), diff(3, 'src/x.rs')]));
    expect(items.map((i) => i.kind)).toEqual(['steps']);
    expect(attached.get(1)?.id).toBe(3);
    expect(attached.has(2)).toBe(false);
  });
  it('leaves a diff nobody asked for as its own item, and never gives one step two diffs', () => {
    const { items, attached } = pairDiffs(turn([edit(1, 'src/x.rs'), diff(2, 'src/z.rs'), diff(3, 'src/x.rs'), diff(4, 'src/x.rs')]));
    expect(items.filter((i) => i.kind === 'diff')).toHaveLength(2);
    expect(attached.get(1)?.id).toBe(3);
  });
});

describe('small helpers', () => {
  it('takes the first letter of a name for the avatar', () => {
    expect(initial('claude · amber')).toBe('C');
    expect(initial('  ')).toBe('?');
  });
  it('writes a short duration with one decimal under ten seconds', () => {
    expect(preciseDuration(4100)).toBe('4.1 s');
    expect(preciseDuration(600)).toBe('0.6 s');
    expect(preciseDuration(0)).toBe('0.0 s');
    expect(preciseDuration(12_000)).toBe('12 s');
    expect(preciseDuration(125_000)).toBe('2 min 5 s');
  });
});
