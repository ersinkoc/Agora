import { describe, expect, it } from 'vitest';
import { decodeText, formatSize, joinPath } from './files';

describe('files helpers', () => {
  it('decodes text and refuses binary', () => {
    expect(decodeText(new TextEncoder().encode('héllo\n'))).toBe('héllo\n');
    expect(decodeText(new Uint8Array([104, 0, 105]))).toBeUndefined();
    expect(decodeText(new Uint8Array([0xff, 0xfe, 0x41]))).toBeUndefined();
  });
  it('decodes UTF-16 with a byte-order mark (what Windows PowerShell writes) and nothing else that looks like it', () => {
    const utf16 = (text: string, little: boolean) => {
      const out = [little ? 0xff : 0xfe, little ? 0xfe : 0xff];
      for (let i = 0; i < text.length; i++) {
        const u = text.charCodeAt(i);
        out.push(...(little ? [u & 0xff, u >> 8] : [u >> 8, u & 0xff]));
      }
      return new Uint8Array(out);
    };
    expect(decodeText(utf16('hello\r\n', true))).toBe('hello\r\n');
    expect(decodeText(utf16('héllo 😀\n', false))).toBe('héllo 😀\n');
    // An odd length, a lone surrogate, a NUL or another control character: not text.
    expect(decodeText(new Uint8Array([0xff, 0xfe, 0x68, 0x00, 0x69]))).toBeUndefined();
    expect(decodeText(new Uint8Array([0xff, 0xfe, 0x00, 0xd8, 0x68, 0x00]))).toBeUndefined();
    expect(decodeText(utf16('a\0b', true))).toBeUndefined();
    expect(decodeText(utf16('a\u0001b', true))).toBeUndefined();
    // Without a byte-order mark the NUL rule stands.
    expect(decodeText(new Uint8Array([0x68, 0x00, 0x69, 0x00]))).toBeUndefined();
  });
  it('joins paths and formats sizes', () => {
    expect(joinPath('', 'a')).toBe('a');
    expect(joinPath('src', 'lib.rs')).toBe('src/lib.rs');
    expect(formatSize(12)).toBe('12 B');
    expect(formatSize(2048)).toBe('2.0 KB');
  });
});
