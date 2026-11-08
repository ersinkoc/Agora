import { describe, expect, it } from 'vitest';
import { commitsOf } from './history';

describe('commitsOf', () => {
  it('fills gaps and reads 64-bit times', () => {
    expect(commitsOf([{ id: 'abc', subject: 's', author: 'a', atMs: 5 }, { atMs: BigInt(7) }])).toEqual([
      { id: 'abc', subject: 's', author: 'a', atMs: 5 },
      { id: '', subject: '', author: '', atMs: 7 },
    ]);
    expect(commitsOf(undefined)).toEqual([]);
  });
});
