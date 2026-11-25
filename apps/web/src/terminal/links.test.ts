import { describe, expect, it } from 'vitest';
import { afterEach, vi } from 'vitest';
import { clipboardOffer, MAX_OFFER, OFFER_GAP_MS, OfferGate, sanitizePaste } from './clipboard';
import { LINK_FEATURES, openLink, safeLink } from './links';

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
  it('drops an offer with a line break, a control, a bidirectional or an invisible character', () => {
    for (const bad of ['a\nb', 'a\rb', 'a\tb', 'ok\x1b[0m', 'a\u202eb', 'a\u200bb', 'a\u2028b', 'a\ufeffb', 'a\u0085b', 'a\u2800b', 'a\u3164b', 'a\uffa0b']) expect(clipboardOffer(`c;${b64(bad)}`), JSON.stringify(bad)).toBeUndefined();
  });
  it('drops an offer that is too large', () => {
    expect(MAX_OFFER).toBe(8192);
    expect(clipboardOffer(`c;${b64('a'.repeat(MAX_OFFER + 1))}`)).toBeUndefined();
    expect(clipboardOffer(`c;${b64('a'.repeat(MAX_OFFER))}`)?.length).toBe(MAX_OFFER);
  });
});

describe('offers one at a time', () => {
  it('lets one through, then none while it is pending or for two seconds', () => {
    const g = new OfferGate();
    expect(g.accept(10_000, false)).toBe(true);
    expect(g.accept(10_001, false)).toBe(false);
    expect(g.accept(10_000 + OFFER_GAP_MS - 1, false)).toBe(false);
    expect(g.accept(10_000 + OFFER_GAP_MS, true)).toBe(false); // pending: never replaced
    expect(g.accept(10_000 + OFFER_GAP_MS, false)).toBe(true);
    // 2000 offers in 0.8 s: one gets through
    const h = new OfferGate();
    let n = 0;
    for (let i = 0; i < 2000; i++) if (h.accept(i * 0.4, false)) n++;
    expect(n).toBe(1);
  });
});

describe('pasted text', () => {
  it('cannot end a bracketed paste early or carry any escape sequence', () => {
    const evil = 'a\x1b[201~rm -rf ~\nb';
    const clean = sanitizePaste(evil);
    expect(clean).toBe('a[201~rm -rf ~\nb');
    expect(clean).not.toContain('\x1b');
    expect(sanitizePaste('x\x1b[200~y\x07\x00\x7f\u009b31m')).toBe('x[200~y31m');
  });
  it('keeps text, tabs and line breaks', () => {
    expect(sanitizePaste('héllo\tworld\r\nnext ✓')).toBe('héllo\tworld\r\nnext ✓');
  });
});

describe('opening a link', () => {
  afterEach(() => vi.unstubAllGlobals());
  it('opens a web address in a new tab with no opener and no referrer', () => {
    const open = vi.fn();
    vi.stubGlobal('window', { open });
    openLink('https://example.com/a');
    expect(open).toHaveBeenCalledWith('https://example.com/a', '_blank', 'noopener,noreferrer');
    expect(LINK_FEATURES.split(',').sort()).toEqual(['noopener', 'noreferrer']);
  });
  it('opens nothing for anything else', () => {
    const open = vi.fn();
    vi.stubGlobal('window', { open });
    openLink('javascript:alert(1)');
    expect(open).not.toHaveBeenCalled();
  });
});
