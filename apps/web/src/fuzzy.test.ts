import { describe, expect, it } from 'vitest';
import { fuzzyScore, rank } from './fuzzy';

describe('fuzzy', () => {
  it('matches subsequences in order and rejects others', () => {
    expect(fuzzyScore('clu', 'claude · amber')).not.toBeNull();
    expect(fuzzyScore('xyz', 'claude')).toBeNull();
    expect(fuzzyScore('ec', 'claude')).toBeNull(); // wrong order
  });
  it('empty query matches everything equally', () => {
    expect(fuzzyScore('', 'anything')).toBe(0);
    expect(fuzzyScore('   ', 'anything')).toBe(0);
  });
  it('prefers word starts and consecutive runs', () => {
    expect(rank(['api tests', 'rapid test'], 'at', (s) => s)[0]).toBe('api tests');
    expect(rank(['c-l-a', 'cla'], 'cla', (s) => s)[0]).toBe('cla');
  });
  it('keeps the original order for equal scores', () => {
    expect(rank(['a1', 'a2', 'a3'], '', (s) => s)).toEqual(['a1', 'a2', 'a3']);
  });
  it('is case-insensitive', () => {
    expect(rank(['Codex Teal'], 'CT', (s) => s)).toEqual(['Codex Teal']);
  });
});
