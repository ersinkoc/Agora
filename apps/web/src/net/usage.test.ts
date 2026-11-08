import { describe, expect, it } from 'vitest';
import { money, summarize } from './usage';

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

  it('formats money and never rounds a real cost down to nothing', () => {
    expect(money(1_240_000, 'en-US')).toBe('$1.24');
    expect(money(2_000, 'en-US')).toBe('<$0.01');
    expect(money(0, 'en-US')).toBe('$0.00');
  });
});
