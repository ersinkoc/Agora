import { describe, expect, it } from 'vitest';
import { freshApprovals, titleWithCount } from './alert';
import type { ApprovalModel } from './view';

const a = (id: string): ApprovalModel => ({ id, node: 'n', localId: id, agentId: 'n/a', agentName: 'x', action: 'run', detail: '', expiresAtMs: 0 });

describe('alert', () => {
  it('finds only approvals not seen before', () => {
    expect(freshApprovals(new Set(['n/1']), [a('n/1'), a('n/2')]).map((x) => x.id)).toEqual(['n/2']);
    expect(freshApprovals(new Set(), [])).toEqual([]);
  });
  it('puts the count in the title only when something waits', () => {
    expect(titleWithCount('Agora', 0)).toBe('Agora');
    expect(titleWithCount('Agora', 2)).toBe('(2) Agora');
  });
});
