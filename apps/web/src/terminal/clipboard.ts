// OSC 52 lets a program put text on the user's clipboard. What a program prints is untrusted (it may be another agent's words or a file
// it was shown), and clipboard text ends up pasted into a shell: the window therefore never writes it by itself. It shows what the
// program offered and copies only when the user presses Copy (docs/security/threat-model.md). A request to READ the clipboard is
// never answered.

export const MAX_OFFER = 100_000;

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
    return text.length > 0 && text.length <= MAX_OFFER ? text : undefined;
  } catch { return undefined; }
}
