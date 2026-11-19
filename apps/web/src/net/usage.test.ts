import { describe, expect, it } from 'vitest';
import { summarize } from './usage';

describe('usage', () => {
  it('keeps only priced agents and sums per machine', () => {
    const s = summarize({
      agents: [
        { agentId: 'n1/a', microUsd: 1_500_000 },
        { agentId: 'n1/b', microUsd: 500_000 },
        { agentId: 'n2/c' }, // no price: absent, not zero
      ],
      totalMicroUsd: 2_000_000,
      pricesAtMs: 0,
    } as never);
    expect(s.perAgent.get('n1/a')).toBe(1_500_000);
    expect(s.perAgent.has('n2/c')).toBe(false);
    expect(s.perNode.get('n1')).toBe(2_000_000);
    expect(s.perNode.has('n2')).toBe(false);
    expect(s.totalMicroUsd).toBe(2_000_000);
  });
});
