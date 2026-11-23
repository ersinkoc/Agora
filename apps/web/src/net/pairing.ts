// What the owner's window knows about machines asking to join (ADR-0065, docs/specs/pairing.md sections 6 and 12). Pure: no DOM, no clock
// (callers pass `now`), no storage. The cards are MEMORY ONLY: the number on a card is what ties it to the machine in front of the person,
// so it is never written to localStorage, never logged, and never put in a page title or a notification.
import { agora } from '../gen/agora.js';
import { EVENT_KIND } from './frame';
import { peerText, type PairRequestModel } from '../components/pairChoice';

/** One request waiting for the owner. `id` is the hub's pair id as lowercase hex (a lookup key, never a secret). */
export interface PairCard extends PairRequestModel {
  /** The hub's clock, milliseconds since the Unix epoch, when the request runs out. */
  expiresMs: number;
  /** The raw pair id the answer names. */
  pairId: Uint8Array;
  /** The hub said this name already belongs to an enrolled machine (`pair.state name_in_use`): then, and only then, Replace is offered. */
  nameTaken?: boolean;
}

export type PairOutcome = 'approved' | 'denied' | 'expired' | 'closed';

/** A request that ended: one line in the console's recent list for a while. */
export interface PairRecent { id: string; kind: 'machine' | 'browser'; name: string; outcome: PairOutcome; at: number }

/** The window's switch, as `PairStatus` answers it. */
export interface PairWindow { open: boolean; closesAtMs: number; pending: number }

/**
 * Whether this session may see cards. Only the hub knows the role: it sends the notices to owner sessions only, and answers `PairStatus`
 * with `auth.forbidden` to anyone else. The client does not trust a notice by itself: cards are shown only once `PairStatus` said yes.
 * `unavailable`: the hub does not offer pairing (`protocol.capability_missing`).
 */
export type PairOwner = 'unknown' | 'yes' | 'no' | 'unavailable';

export interface PairState {
  owner: PairOwner;
  cards: readonly PairCard[];
  recent: readonly PairRecent[];
  window?: PairWindow;
}

export const emptyPairState = (): PairState => ({ owner: 'unknown', cards: [], recent: [] });

/** The hub shows at most 4 cards; so does the window. */
export const MAX_CARDS = 4;
export const MAX_RECENT = 3;
/** How long a finished request's line stays in the recent list. */
export const RECENT_MS = 2 * 60_000;

export type PairNote =
  | { type: 'reset' }
  | { type: 'requested'; card: PairCard }
  | { type: 'decided'; id: string; outcome: PairOutcome; at: number }
  | { type: 'owner'; owner: PairOwner }
  | { type: 'window'; window: PairWindow | undefined }
  | { type: 'name-taken'; id: string };

const hex = (b: Uint8Array): string => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
const OUTCOMES: Record<number, PairOutcome> = { 1: 'approved', 2: 'denied', 3: 'expired', 4: 'closed' };

/**
 * Reads one events-stream frame as a pairing notice; `undefined` for any other frame and for one that is malformed (a bad notice is
 * dropped, never half shown: no id, an unknown kind, a number that is not six digits, no name).
 */
export function decodePairFrame(kind: number, payload: Uint8Array, at: number): PairNote | undefined {
  try {
    if (kind === EVENT_KIND.PAIR_REQUESTED) {
      const m = agora.v1.PairRequested.decode(payload);
      const id = m.pairId ?? new Uint8Array();
      const k = m.kind === agora.v1.PairKind.PAIR_KIND_MACHINE ? 'machine' : m.kind === agora.v1.PairKind.PAIR_KIND_BROWSER ? 'browser' : undefined;
      if (!k || id.length === 0 || id.length > 32 || !/^\d{6}$/.test(m.number ?? '') || !peerText(m.name)) return undefined;
      const expires = typeof m.expiresMs === 'number' ? m.expiresMs : Number(m.expiresMs?.toString() ?? 0);
      return { type: 'requested', card: { id: hex(id), pairId: id, kind: k, name: peerText(m.name), os: peerText(m.os ?? ''), address: peerText(m.address ?? ''), number: m.number ?? '', expiresMs: expires } };
    }
    if (kind === EVENT_KIND.PAIR_DECIDED) {
      const m = agora.v1.PairDecided.decode(payload);
      const id = m.pairId ?? new Uint8Array();
      const outcome = OUTCOMES[m.outcome ?? 0];
      if (id.length === 0 || id.length > 32 || !outcome) return undefined;
      return { type: 'decided', id: hex(id), outcome, at };
    }
  } catch { /* a garbled notice is dropped */ }
  return undefined;
}

/** Applies one notice. Never mutates its input. */
export function applyPairNote(s: PairState, n: PairNote): PairState {
  switch (n.type) {
    case 'owner':
      // A session that is not the owner's holds nothing: whatever arrived before the answer is forgotten.
      return n.owner === 'yes' ? { ...s, owner: n.owner } : { ...s, owner: n.owner, cards: [], window: undefined };
    case 'window':
      return { ...s, window: n.window };
    case 'reset':
      // A new connection: the hub sends the cards waiting now, so what this window held is stale until it says so again.
      return { ...s, cards: [] };
    case 'requested': {
      if (s.owner === 'no' || s.owner === 'unavailable') return s;
      const rest = s.cards.filter((c) => c.id !== n.card.id);
      const old = s.cards.find((c) => c.id === n.card.id);
      const next = [...rest, { ...n.card, nameTaken: old?.nameTaken }];
      return { ...s, cards: next.slice(-MAX_CARDS) };
    }
    case 'name-taken':
      return { ...s, cards: s.cards.map((c) => (c.id === n.id ? { ...c, nameTaken: true } : c)) };
    case 'decided': {
      // One machine per opening (#942): the hub closes the switch when it takes an approval, from any window or `agora pair open`,
      // so the switch is shown closed at once instead of at the next `PairStatus`.
      const t = n.outcome === 'approved' && s.window?.open ? { ...s, window: { open: false, closesAtMs: 0, pending: 0 } } : s;
      const card = t.cards.find((c) => c.id === n.id);
      if (!card) return t;
      const line: PairRecent = { id: card.id, kind: card.kind, name: card.name, outcome: n.outcome, at: n.at };
      return { ...t, cards: t.cards.filter((c) => c.id !== n.id), recent: [line, ...t.recent].slice(0, MAX_RECENT) };
    }
  }
}

/** The cards to show: none unless the hub confirmed this session is the owner's. */
export const shownCards = (s: PairState): readonly PairCard[] => (s.owner === 'yes' ? s.cards : []);

/** The recent lines still worth showing at `now`. */
export const shownRecent = (s: PairState, now: number): readonly PairRecent[] =>
  s.owner === 'yes' ? s.recent.filter((r) => now - r.at < RECENT_MS) : [];

/** Seconds a request has left at `now` by the hub's clock; 0 when it has run out. */
export const secondsLeft = (c: Pick<PairCard, 'expiresMs'>, now: number): number => Math.max(0, Math.ceil((c.expiresMs - now) / 1000));
