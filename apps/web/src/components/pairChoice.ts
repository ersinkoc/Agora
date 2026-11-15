import { t } from '../i18n';

/** ADR-0065: who asks to pair. A machine wants to join; a browser on another machine wants to sign in. */
export interface PairRequestModel {
  id: string;
  kind: 'machine' | 'browser';
  /** The machine's name, or the browser's summary (`Chrome · macOS`). */
  name: string;
  /** A machine's system as it reports it (`windows`, `macos`, `linux`); empty or absent when unknown, and for a browser. */
  os?: string;
  /** The address the request came from, as the hub saw it. */
  address: string;
  /** The number both screens show (ADR-0065 decision 4): 6 digits. */
  number: string;
}

/** ADR-0065: a sign-in that waits for the owner's approval, as the browser sees it. */
export interface SignInApprovalModel {
  number: string;
  state: 'waiting' | 'denied' | 'expired';
}

/** The 6-digit number as `482-193`, read in two halves the way the other screen shows it; anything else as given. */
export function formatPairNumber(n: string): string {
  return /^\d{6}$/.test(n) ? `${n.slice(0, 3)}-${n.slice(3)}` : n;
}

/** A machine's system as a person reads it (`Windows`); a system this build has no words for is shown as reported. */
export function systemName(os: string | undefined): string {
  if (!os) return '';
  const key = `os-${os}`;
  const text = t(key);
  return text === key ? os : text;
}

/**
 * The card's title: one fixed sentence. The name and the system come from the machine or browser that asks, so they are NOT built into a
 * sentence with the address (a name such as `x" (linux, 10.0.0.1) wants to join. New machine "y` would pass for the address): each
 * has its own labelled row on the card, and only the address is the hub's own observation.
 */
export function pairTitle(r: PairRequestModel): string {
  return t(r.kind === 'browser' ? 'pair-browser-title' : 'pair-machine-title');
}
