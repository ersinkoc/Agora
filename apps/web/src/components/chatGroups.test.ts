import { describe, expect, it } from 'vitest';
import type { ChatLine } from '../net/chat';
import { groupLines } from './ChatEntry';

const l = (id: number, kind: ChatLine['kind'], role: ChatLine['role'] = 'agent'): ChatLine => ({ id, atMs: 0, role, kind, text: '', path: '' });

describe('conversation groups', () => {
  it('puts steps in a row together and opens only the latest plan', () => {
    const g = groupLines([l(1, 'text', 'user'), l(2, 'plan'), l(3, 'tool'), l(4, 'tool'), l(5, 'diff'), l(6, 'tool'), l(7, 'plan'), l(8, 'text')]);
    expect(g.map((x) => x.kind === 'steps' ? `steps:${x.lines.map((y) => y.id).join(',')}` : `${x.line.id}${x.latestPlan ? '*' : ''}`))
      .toEqual(['1', '2', 'steps:3,4', '5', 'steps:6', '7*', '8']);
  });
  it('is empty for no lines', () => expect(groupLines([])).toEqual([]));
});
