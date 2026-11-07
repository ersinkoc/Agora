import { describe, expect, it } from 'vitest';
import { type AgentTileModel, needsAttention, sortFleet, uiState } from './model';

const tile = (id: string, state: AgentTileModel['state']): AgentTileModel => ({
  id, name: id, accent: '#fff', machine: 'm', workspace: 'w', state, preview: '',
});

describe('model', () => {
  it('maps protocol states, treating unknown values as unknown', () => {
    expect([1, 2, 3, 4, 0, 99, undefined].map(uiState)).toEqual(
      ['working', 'needs-you', 'idle', 'finished', 'unknown', 'unknown', 'unknown']);
  });
  it('lists only agents that need the user', () => {
    const a = [tile('a', 'working'), tile('b', 'needs-you')];
    expect(needsAttention(a).map((x) => x.id)).toEqual(['b']);
  });
  it('sorts most urgent first and keeps order inside a group', () => {
    const a = [tile('1', 'finished'), tile('2', 'idle'), tile('3', 'needs-you'), tile('4', 'idle'), tile('5', 'working')];
    expect(sortFleet(a).map((x) => x.id)).toEqual(['3', '5', '2', '4', '1']);
  });
});
