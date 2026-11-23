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

  it('has one fixed title and gives the name, the system and the address a row each, leaving out a system it does not know', () => {
    expect(pairTitle(machine)).toBe('A new machine wants to join');
    expect(pairTitle(browser)).toBe('A browser wants to sign in');
    const m = renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => 'ok'} />);
    expect(m).toContain('<dt class="text-muted">Name</dt><dd class="m-0 min-w-0 break-words text-ink"><bdi>laptop</bdi></dd>');
    expect(m).toContain('<bdi>Windows</bdi>');
    expect(m).toContain('>192.168.1.50</bdi></dd>');
    expect(renderToStaticMarkup(<PairRequest request={{ ...machine, os: 'macos' }} onAnswer={async () => 'ok'} />)).toContain('<bdi>macOS</bdi>');
    // No system known: no System row.
    for (const os of ['', undefined]) {
      expect(renderToStaticMarkup(<PairRequest request={{ ...machine, os }} onAnswer={async () => 'ok'} />)).not.toContain('>System<');
    }
    // A system this build has no words for is shown as the machine reported it.
    expect(systemName('freebsd')).toBe('freebsd');
    // A browser has no system row.
    expect(renderToStaticMarkup(<PairRequest request={browser} onAnswer={async () => 'ok'} />)).not.toContain('>System<');
  });

  it('keeps a name that imitates an address in the name row: the address is its own row, what the hub saw', () => {
    const hostile = { ...machine, name: 'x" (linux, 10.0.0.1) wants to join. New machine "y' };
    const html = renderToStaticMarkup(<PairRequest request={hostile} onAnswer={async () => 'ok'} />);
    // Text, never markup, and only inside the name row.
    expect(html).toContain('<bdi>x&quot; (linux, 10.0.0.1) wants to join. New machine &quot;y</bdi>');
    expect(html.split('10.0.0.1').length - 1).toBe(1);
    // The title does not carry the name, and the real address is in the address row.
    expect(html).toContain('>A new machine wants to join</h2>');
    expect(html).toContain('>192.168.1.50</bdi></dd>');
    expect(pairTitle(hostile)).not.toContain('10.0.0.1');
    // Markup in a name is escaped.
    const tag = renderToStaticMarkup(<PairRequest request={{ ...machine, name: '<img src=x onerror=alert(1)>' }} onAnswer={async () => 'ok'} />);
    expect(tag).not.toContain('<img');
    expect(tag).toContain('&lt;img');
  });

  it('puts the number and what to compare it with on the card, with Deny and "Numbers match…"', () => {
    const m = renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => 'ok'} />);
    expect(m).toContain('<span>482</span><span>193</span>');
    expect(m).toContain('Compare with the number on the new machine. Approve only if they are the same.');
    expect(m).toContain('>Deny<');
    expect(m).toContain('>Numbers match…<');
    // The number comes before the buttons that answer it.
    expect(m.indexOf('<span>482</span>')).toBeLessThan(m.indexOf('>Deny<'));
    // The card is described by the comparison sentence, so a screen reader reads it with the title.
    expect(m).toContain('aria-describedby="pair-request-p1-compare"');
    expect(m).toContain('id="pair-request-p1-compare"');
    const b = renderToStaticMarkup(<PairRequest request={browser} onAnswer={async () => 'ok'} />);
    expect(b).toContain('<span>731</span><span>004</span>');
    expect(b).toContain('Compare with the number on that browser. Approve only if they are the same.');
  });

  it('floats over the window unless shown in place', () => {
    expect(renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => 'ok'} />)).toContain('fixed bottom-4');
    expect(renderToStaticMarkup(<PairRequest inline request={machine} onAnswer={async () => 'ok'} />)).not.toContain('fixed bottom-4');
  });

  it('uses no internal word on the card or the sign-in page', () => {
    const html = [
      renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => 'ok'} />),
      renderToStaticMarkup(<PairRequest request={browser} onAnswer={async () => 'ok'} />),
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
    expect(renderToStaticMarkup(<SignInPage onSubmit={() => {}} onPasskey={() => {}} approval={{ number: '731004', state: 'waiting' }} />)).toContain('Use a passkey');
  });

  it('speaks Turkish', () => {
    setLanguage('tr');
    expect(pairTitle(machine)).toBe('Yeni bir makine katılmak istiyor');
    expect(renderToStaticMarkup(<PairRequest request={machine} onAnswer={async () => 'ok'} />)).toContain('<dt class="text-muted">Adres</dt>');
    expect(renderToStaticMarkup(<SignInWaiting approval={{ number: '731004', state: 'waiting' }} onAskAgain={() => {}} onUseCode={() => {}} />)).toContain('Bunun yerine kod kullan');
    setLanguage('en');
  });
});
