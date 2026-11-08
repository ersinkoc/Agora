import { describe, expect, it } from 'vitest';
import { decodeText, formatSize, joinPath } from './files';

describe('files helpers', () => {
  it('decodes text and refuses binary', () => {
    expect(decodeText(new TextEncoder().encode('héllo\n'))).toBe('héllo\n');
    expect(decodeText(new Uint8Array([104, 0, 105]))).toBeUndefined();
    expect(decodeText(new Uint8Array([0xff, 0xfe, 0x41]))).toBeUndefined();
  });
  it('joins paths and formats sizes', () => {
    expect(joinPath('', 'a')).toBe('a');
    expect(joinPath('src', 'lib.rs')).toBe('src/lib.rs');
    expect(formatSize(12)).toBe('12 B');
    expect(formatSize(2048)).toBe('2.0 KB');
  });
});
