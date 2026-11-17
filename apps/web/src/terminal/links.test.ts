import { describe, expect, it } from 'vitest';
import { clipboardOffer, MAX_OFFER } from './clipboard';
import { safeLink } from './links';

describe('links a program can put on text', () => {
  it('opens only web addresses', () => {
    expect(safeLink('https://example.com/a?b=1')).toBe('https://example.com/a?b=1');
    expect(safeLink('http://localhost:3000')).toBe('http://localhost:3000/');
    for (const bad of ['javascript:alert(1)', 'data:text/html,x', 'file:///etc/passwd', 'ssh://host', 'vbscript:x', '//example.com', 'example.com', '', 'https://user:pw@example.com/', 'JaVaScRiPt:alert(1)']) expect(safeLink(bad), bad).toBeUndefined();
  });
  it('refuses an address that is far too long', () => {
    expect(safeLink(`https://example.com/${'a'.repeat(3000)}`)).toBeUndefined();
  });
});

describe('OSC 52 clipboard offers', () => {
  const b64 = (s: string) => btoa(String.fromCharCode(...new TextEncoder().encode(s)));
  it('decodes what a program offers', () => {
    expect(clipboardOffer(`c;${b64('hello')}`)).toBe('hello');
    expect(clipboardOffer(`;${b64('héllo ✓')}`)).toBe('héllo ✓');
    expect(clipboardOffer(`pc;${b64('x')}`)).toBe('x');
  });
  it('never answers a read request and drops anything malformed', () => {
    for (const bad of ['c;?', 'c;', 'c', 'c;!!!', 'c;%%%%', `c;${btoa('\xff\xfe')}`]) expect(clipboardOffer(bad), bad).toBeUndefined();
  });
  it('drops an offer that is too large', () => {
    expect(clipboardOffer(`c;${b64('a'.repeat(MAX_OFFER + 1))}`)).toBeUndefined();
    expect(clipboardOffer(`c;${b64('a'.repeat(MAX_OFFER))}`)?.length).toBe(MAX_OFFER);
  });
});
