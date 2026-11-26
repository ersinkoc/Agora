// OSC 52 lets a program put text on the user's clipboard. What a program prints is untrusted (it may be another agent's words or a file
// it was shown), and clipboard text ends up pasted into a shell: the window therefore never writes it by itself. It shows what the
// program offered and copies only when the user presses Copy (docs/security/threat-model.md). A request to READ the clipboard is
// never answered.

import { hasHidden } from '../net/hidden';

export const MAX_OFFER = 8192;
/** At most one new offer per this many milliseconds; one that is still pending is never replaced. */
export const OFFER_GAP_MS = 2000;

// Control characters (a newline included: a line break would hide a second command after what is shown), bidirectional controls and
// invisible format characters (they make a text read differently from what it does) make an offer unshowable: it is dropped.
const UNSHOWABLE = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u;

/** The text a `52;Pc;Pd` request offers, or `undefined` for a read request, anything malformed, an empty text or one too large. */
export function clipboardOffer(data: string): string | undefined {
  const semi = data.indexOf(';');
  if (semi < 0) return undefined;
  const payload = data.slice(semi + 1);
  if (payload === '?' || payload === '') return undefined;
  if (payload.length > Math.ceil(MAX_OFFER * 4 / 3) + 4 || !/^[A-Za-z0-9+/]*={0,2}$/.test(payload)) return undefined;
  try {
    const bin = atob(payload);
    const text = new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
    return text.length > 0 && text.length <= MAX_OFFER && !UNSHOWABLE.test(text) && !hasHidden(text) ? text : undefined;
  } catch { return undefined; }
}

/** Lets an offer through only when none is pending and the last one was long enough ago. */
export class OfferGate {
  private last = Number.NEGATIVE_INFINITY;
  accept(now: number, pending: boolean): boolean {
    if (pending || now - this.last < OFFER_GAP_MS) return false;
    this.last = now;
    return true;
  }
}

// Text pasted into a terminal reaches a program as typed. With bracketed paste on, the program trusts the end marker `ESC [ 201 ~`, so
// pasted text that carries it (or any escape sequence) could end the paste early and have the rest run as typed commands. No escape
// and no other control character survives except tab and line breaks; C1 controls go too.
export function sanitizePaste(text: string): string {
  return text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, '');
}
