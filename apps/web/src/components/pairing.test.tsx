import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import { SignInPage } from '../screens';
import { PairRequest } from './PairRequest';
import { SignInWaiting } from './SignInWaiting';
import { formatPairNumber, pairTitle, systemName, type PairRequestModel } from './pairChoice';

const machine: PairRequestModel = { id: 'p1', kind: 'machine', name: 'laptop', os: 'windows', address: '192.168.1.50', number: '482193' };
const browser: PairRequestModel = { id: 'p2', kind: 'browser', name: 'Chrome · macOS', address: '192.168.1.57', number: '731004' };

describe('pairing (ADR-0065)', () => {
  beforeEach(() => setLanguage('en'));

  it('shows the number in two halves, the way the other screen does', () => {
    expect(formatPairNumber('482193')).toBe('482-193');
    expect(formatPairNumber('004017')).toBe('004-017');
    // Anything that is not six digits is shown as sent, never padded or cut.
    expect(formatPairNumber('48219')).toBe('48219');
    expect(formatPairNumber('482-193')).toBe('482-193');
  });

  it('names a machine with its system and address, and leaves out a system it does not know', () => {
    expect(pairTitle(machine)).toBe('New machine "laptop" (Windows, 192.168.1.50) wants to join');
    expect(pairTitle({ ...machine, os: 'macos' })).toContain('(macOS, 192.168.1.50)');
    expect(pairTitle({ ...machine, os: '' })).toBe('New machine "laptop" (192.168.1.50) wants to join');
    expect(pairTitle({ ...machine, os: undefined })).toBe('New machine "laptop" (192.168.1.50) wants to join');
    // A system this build has no words for is shown as the machine reported it.
    expect(systemName('freebsd')).toBe('freebsd');
    expect(pairTitle(browser)).toBe('Browser "Chrome · macOS" (192.168.1.57) wants to sign in');
  });

  it('puts the number and what to compare it with on the card, with Deny and Approve', () => {
    const m = renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => true} />);
    expect(m).toContain('482-193');
    expect(m).toContain('Approve only if the new machine shows the same number.');
    expect(m).toContain('>Deny<');
    expect(m).toContain('>Approve<');
    // The card is described by the comparison sentence, so a screen reader reads it with the title.
    expect(m).toContain('aria-describedby="pair-request-p1-compare"');
    expect(m).toContain('id="pair-request-p1-compare"');
    const b = renderToStaticMarkup(<PairRequest request={browser} onAnswer={async () => true} />);
    expect(b).toContain('731-004');
    expect(b).toContain('Approve only if that browser shows the same number.');
  });

  it('floats over the window unless shown in place', () => {
    expect(renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => true} />)).toContain('fixed bottom-4');
    expect(renderToStaticMarkup(<PairRequest inline request={machine} onAnswer={async () => true} />)).not.toContain('fixed bottom-4');
  });

  it('uses no internal word on the card or the sign-in page', () => {
    const html = [
      renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => true} />),
      renderToStaticMarkup(<PairRequest request={browser} onAnswer={async () => true} />),
      ...(['waiting', 'denied', 'expired'] as const).map((state) =>
        renderToStaticMarkup(<SignInWaiting approval={{ number: '731004', state }} onAskAgain={() => {}} onUseCode={() => {}} />)),
    ].join(' ');
    for (const word of [/\bnode\b/i, /\bhub\b/i, /\benrol/i, /\bpairing\b/i]) expect(html).not.toMatch(word);
  });
});

describe('signing in by approval (ADR-0065 decision 6)', () => {
  beforeEach(() => setLanguage('en'));

  it('shows the number while it waits, and a code is one click away', () => {
    const html = renderToStaticMarkup(<SignInWaiting approval={{ number: '731004', state: 'waiting' }} onAskAgain={() => {}} onUseCode={() => {}} />);
    expect(html).toContain('Waiting for the owner to approve this browser…');
    expect(html).toContain('731-004');
    expect(html).toContain('Use a code instead');
    // Nothing to ask again for while the request is still open.
    expect(html).not.toContain('Ask again');
    expect(html).toContain('aria-live="polite"');
  });

  it('says what happened when the owner says no or nobody answers, and offers to ask again', () => {
    const denied = renderToStaticMarkup(<SignInWaiting approval={{ number: '731004', state: 'denied' }} onAskAgain={() => {}} onUseCode={() => {}} />);
    expect(denied).toContain('The owner did not approve this browser.');
    expect(denied).toContain('Ask again');
    // The old number is gone: asking again is a new request with a new number.
    expect(denied).not.toContain('731-004');
    const expired = renderToStaticMarkup(<SignInWaiting approval={{ number: '731004', state: 'expired' }} onAskAgain={() => {}} onUseCode={() => {}} />);
    expect(expired).toContain('Nobody approved this browser in time.');
  });

  it('shows the code form on the sign-in page only when no approval is pending', () => {
    const byCode = renderToStaticMarkup(<SignInPage onSubmit={() => {}} />);
    expect(byCode).toContain('agora hub login');
    expect(byCode).not.toContain('Waiting for the owner');
    const waiting = renderToStaticMarkup(<SignInPage onSubmit={() => {}} approval={{ number: '731004', state: 'waiting' }} />);
    expect(waiting).toContain('731-004');
    expect(waiting).not.toContain('<form');
    // A passkey still works while it waits.
    expect(renderToStaticMarkup(<SignInPage onSubmit={() => {}} onPasskey={() => {}} approval={{ number: '731004', state: 'waiting' }} />)).toContain('Sign in with a passkey');
  });

  it('speaks Turkish', () => {
    setLanguage('tr');
    expect(pairTitle(machine)).toBe('Yeni makine "laptop" (Windows, 192.168.1.50) katılmak istiyor');
    expect(renderToStaticMarkup(<SignInWaiting approval={{ number: '731004', state: 'waiting' }} onAskAgain={() => {}} onUseCode={() => {}} />)).toContain('Bunun yerine kod kullan');
    setLanguage('en');
  });
});
