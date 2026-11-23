import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { encodeFrame, EVENT_KIND, STREAM_EVENTS } from './frame';
import { connectLive, type Socket } from './live';
import { applyPairNote, decodePairFrame, emptyPairState, MAX_CARDS, RECENT_MS, secondsLeft, shownCards, shownRecent, type PairCard, type PairNote, type PairState } from './pairing';

const fixture = (name: string) => new Uint8Array(readFileSync(`../../schema/fixtures/v1/agora.v1.${name}.bin`));
const requested = (over: Partial<agora.v1.PairRequested.$Properties> = {}, id = 1): Uint8Array => agora.v1.PairRequested.encode({
  pairId: new Uint8Array(16).fill(id), kind: agora.v1.PairKind.PAIR_KIND_MACHINE, name: 'laptop', os: 'windows', address: '192.168.1.50', number: '482193', expiresMs: 1_000_000, ...over,
}).finish();
const card = (id: string, over: Partial<PairCard> = {}): PairCard => ({ id, pairId: new Uint8Array([1]), kind: 'machine', name: id, os: 'linux', address: '10.0.0.1', number: '111222', expiresMs: 500, ...over });
const owner = (s: PairState = emptyPairState()) => applyPairNote(s, { type: 'owner', owner: 'yes' });
const apply = (s: PairState, ...notes: PairNote[]) => notes.reduce(applyPairNote, s);

afterEach(() => vi.restoreAllMocks());

describe('the pairing notices (ADR-0065)', () => {
  it('reads a card the hub sent, with the pair id as hex and the number as six digits', () => {
    const n = decodePairFrame(EVENT_KIND.PAIR_REQUESTED, requested(), 0);
    expect(n).toMatchObject({ type: 'requested', card: { id: '01'.repeat(16), kind: 'machine', name: 'laptop', os: 'windows', address: '192.168.1.50', number: '482193', expiresMs: 1_000_000 } });
  });

  it('reads the published fixtures: the decision, and a card whose number is not six digits is dropped, never half shown', () => {
    expect(decodePairFrame(EVENT_KIND.PAIR_DECIDED, fixture('PairDecided'), 7)).toEqual({ type: 'decided', id: '010203', outcome: 'approved', at: 7 });
    // The fixture of PairRequested carries the placeholder number "x": a client must not show it.
    expect(decodePairFrame(EVENT_KIND.PAIR_REQUESTED, fixture('PairRequested'), 0)).toBeUndefined();
  });

  it('drops what is malformed: no id, an unknown kind, no name, a long number, garbage, and kinds that are not pairing', () => {
    expect(decodePairFrame(EVENT_KIND.PAIR_REQUESTED, requested({ pairId: new Uint8Array() }), 0)).toBeUndefined();
    expect(decodePairFrame(EVENT_KIND.PAIR_REQUESTED, requested({ kind: agora.v1.PairKind.PAIR_KIND_UNSPECIFIED }), 0)).toBeUndefined();
    expect(decodePairFrame(EVENT_KIND.PAIR_REQUESTED, requested({ name: '' }), 0)).toBeUndefined();
    expect(decodePairFrame(EVENT_KIND.PAIR_REQUESTED, requested({ number: '4821930' }), 0)).toBeUndefined();
    expect(decodePairFrame(EVENT_KIND.PAIR_REQUESTED, new Uint8Array([0xff, 0xff, 0xff]), 0)).toBeUndefined();
    expect(decodePairFrame(EVENT_KIND.PAIR_DECIDED, agora.v1.PairDecided.encode({ pairId: new Uint8Array([1]), outcome: agora.v1.PairOutcome.PAIR_OUTCOME_UNSPECIFIED }).finish(), 0)).toBeUndefined();
    expect(decodePairFrame(EVENT_KIND.SNAPSHOT, requested(), 0)).toBeUndefined();
  });

  it('shows no card until the hub has said this session is the owner\'s, and forgets what it held if it is not', () => {
    const held = apply(emptyPairState(), { type: 'requested', card: card('a') });
    expect(held.cards).toHaveLength(1);
    expect(shownCards(held)).toEqual([]);
    expect(shownCards(owner(held)).map((c) => c.id)).toEqual(['a']);
    const no = applyPairNote(held, { type: 'owner', owner: 'no' });
    expect(no.cards).toEqual([]);
    // And a session that is not the owner's keeps nothing that arrives later either.
    expect(applyPairNote(no, { type: 'requested', card: card('b') }).cards).toEqual([]);
    expect(applyPairNote(applyPairNote(held, { type: 'owner', owner: 'unavailable' }), { type: 'requested', card: card('b') }).cards).toEqual([]);
  });

  it('keeps at most four cards, one per request, and the newest wins', () => {
    let s = owner();
    for (const id of ['a', 'b', 'c', 'd', 'e']) s = applyPairNote(s, { type: 'requested', card: card(id) });
    expect(s.cards.map((c) => c.id)).toEqual(['b', 'c', 'd', 'e']);
    expect(s.cards).toHaveLength(MAX_CARDS);
    s = applyPairNote(s, { type: 'requested', card: card('c', { name: 'again' }) });
    expect(s.cards.filter((c) => c.id === 'c')).toHaveLength(1);
  });

  it('ends a card with its outcome and leaves one line for the recent list', () => {
    let s = apply(owner(), { type: 'requested', card: card('a', { name: 'laptop' }) }, { type: 'requested', card: card('b') });
    s = applyPairNote(s, { type: 'decided', id: 'a', outcome: 'approved', at: 1000 });
    expect(s.cards.map((c) => c.id)).toEqual(['b']);
    expect(s.recent).toEqual([{ id: 'a', kind: 'machine', name: 'laptop', outcome: 'approved', at: 1000 }]);
    // An outcome for a request this window never held says nothing.
    expect(applyPairNote(s, { type: 'decided', id: 'zzz', outcome: 'denied', at: 2 })).toBe(s);
    for (const outcome of ['denied', 'expired', 'closed'] as const) {
      expect(applyPairNote(apply(owner(), { type: 'requested', card: card('x') }), { type: 'decided', id: 'x', outcome, at: 5 }).recent[0]?.outcome).toBe(outcome);
    }
    // The line is shown for a while, then it is gone.
    expect(shownRecent(s, 1000 + RECENT_MS - 1)).toHaveLength(1);
    expect(shownRecent(s, 1000 + RECENT_MS)).toHaveLength(0);
  });

  it('shows the switch closed once a machine was approved, wherever it was approved (#942: one machine per opening)', () => {
    const open = { open: true, closesAtMs: 600_000, pending: 2 };
    let s = apply(owner(), { type: 'window', window: open }, { type: 'requested', card: card('a') }, { type: 'requested', card: card('b') });
    // A deny, an expiry or a closed request leaves the switch as it was.
    for (const outcome of ['denied', 'expired', 'closed'] as const) {
      expect(applyPairNote(s, { type: 'decided', id: 'a', outcome, at: 5 }).window).toEqual(open);
    }
    s = applyPairNote(s, { type: 'decided', id: 'a', outcome: 'approved', at: 5 });
    expect(s.window).toEqual({ open: false, closesAtMs: 0, pending: 0 });
    // Also for an approval of a request this window never held (another window, `agora pair open`).
    const other = applyPairNote(apply(owner(), { type: 'window', window: open }), { type: 'decided', id: 'zzz', outcome: 'approved', at: 5 });
    expect(other.window).toEqual({ open: false, closesAtMs: 0, pending: 0 });
  });

  it('removes stale cards when the connection starts over: only what the hub sends again is shown', () => {
    let s = apply(owner(), { type: 'requested', card: card('a') }, { type: 'requested', card: card('b') });
    s = applyPairNote(s, { type: 'reset' });
    expect(s.cards).toEqual([]);
    expect(s.owner).toBe('yes');
    s = applyPairNote(s, { type: 'requested', card: card('b') });
    expect(s.cards.map((c) => c.id)).toEqual(['b']);
  });

  it('remembers that a name is taken, also when the hub sends the same card again', () => {
    let s = apply(owner(), { type: 'requested', card: card('a') }, { type: 'name-taken', id: 'a' });
    expect(s.cards[0]?.nameTaken).toBe(true);
    s = applyPairNote(s, { type: 'requested', card: card('a') });
    expect(s.cards[0]?.nameTaken).toBe(true);
    // A card that never got a refusal has no Replace.
    expect(apply(owner(), { type: 'requested', card: card('b') }).cards[0]?.nameTaken).toBeUndefined();
  });

  it('counts down from the hub\'s expiry and stops at zero', () => {
    expect(secondsLeft({ expiresMs: 10_500 }, 10_000)).toBe(1);
    expect(secondsLeft({ expiresMs: 10_000 }, 10_000)).toBe(0);
    expect(secondsLeft({ expiresMs: 10_000 }, 99_999)).toBe(0);
  });

  it('writes nothing anywhere: no storage, no console, no page title', () => {
    const store = { setItem: vi.fn(), getItem: vi.fn(), removeItem: vi.fn() };
    vi.stubGlobal('localStorage', store);
    vi.stubGlobal('sessionStorage', store);
    const spies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) => vi.spyOn(console, m));
    const n = decodePairFrame(EVENT_KIND.PAIR_REQUESTED, requested(), 0);
    let s = owner();
    if (n) s = applyPairNote(s, n);
    s = applyPairNote(s, { type: 'decided', id: '01'.repeat(16), outcome: 'approved', at: 1 });
    shownCards(s); shownRecent(s, 2);
    expect(store.setItem).not.toHaveBeenCalled();
    for (const spy of spies) expect(spy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});

class FakeSocket implements Socket {
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  constructor(readonly url: string, readonly protocols: string[]) {}
  send(d: Uint8Array) { this.sent.push(d); }
  close() { this.onclose?.(); }
  deliver(kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId: STREAM_EVENTS, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
}

describe('the live link and the pairing notices', () => {
  it('hands the notices to the window, starts each connection with a reset, and still ignores kinds it does not know', () => {
    const sockets: FakeSocket[] = [];
    const notes: PairNote[] = [];
    const live = connectLive({ url: 'ws://hub/', token: 't', backoffMs: () => 0, wake: () => () => {}, onView: () => {}, onState: () => {}, onPair: (n) => notes.push(n),
      socket: (url, protocols) => { const s = new FakeSocket(url, protocols); sockets.push(s); return s; } });
    const s = sockets[0] as FakeSocket;
    s.onopen?.();
    expect(notes).toEqual([{ type: 'reset' }]);
    s.deliver(EVENT_KIND.PAIR_REQUESTED, requested());
    s.deliver(EVENT_KIND.PAIR_DECIDED, agora.v1.PairDecided.encode({ pairId: new Uint8Array(16).fill(1), outcome: agora.v1.PairOutcome.PAIR_OUTCOME_DENIED }).finish());
    s.deliver(99, new Uint8Array([1, 2, 3]));
    expect(notes.map((n) => n.type)).toEqual(['reset', 'requested', 'decided']);
    // A garbled notice is dropped and does not end the connection.
    s.deliver(EVENT_KIND.PAIR_REQUESTED, new Uint8Array([0xff, 0xff]));
    expect(notes).toHaveLength(3);
    live.close();
  });
});
