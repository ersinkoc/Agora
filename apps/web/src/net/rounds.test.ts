import { describe, expect, it } from 'vitest';
import { Rounds } from './rounds';

describe('rounds of reads', () => {
  it('drops a success and a failure that complete after a newer round started', async () => {
    const r = new Rounds();
    const seen: string[] = [];
    let resolveOld!: (v: string) => void;
    let rejectOld!: (e: Error) => void;
    const oldOk = new Promise<string>((res) => { resolveOld = res; });
    const oldBad = new Promise<string>((_, rej) => { rejectOld = rej; });
    const put = r.bind((v: string) => seen.push(v));
    oldOk.then(put, () => put('failed'));
    oldBad.then(put, () => put('failed'));
    r.next();
    const current = r.bind((v: string) => seen.push(v));
    await Promise.resolve('current text').then(current);
    resolveOld('old text');
    rejectOld(new Error('old'));
    await new Promise((res) => setTimeout(res, 0));
    expect(seen).toEqual(['current text']);
  });

  it('keeps completions of the current round', async () => {
    const r = new Rounds();
    const seen: number[] = [];
    const put = r.bind((v: number) => seen.push(v));
    await Promise.resolve(1).then(put);
    put(2);
    expect(seen).toEqual([1, 2]);
  });
});
