// The live connection to the hub (ADR-0027): subscribe, take the snapshot, follow positioned events, and come back
// by itself with the position it has. A browser cannot set headers on a WebSocket, so the token travels as a
// subprotocol (`agora.v1`, `agora.bearer.<token>`).

import { agora } from '../gen/agora.js';
import { chatLine, type ChatLine, type ChatSource } from './chat';
const CHAT_KIND = { SNAPSHOT: 10, ENTRY: 11 } as const;
import { decodeTerminalFrame, historyPairing, type TerminalEvent } from '../terminal/frames';
import type { TerminalSource } from '../terminal/source';
import { CONTROL_KIND, decodeFrame, encodeFrame, EVENT_KIND, STREAM_COMMANDS, STREAM_CLOSE, STREAM_CONTROL, STREAM_EVENTS } from './frame';
import { decodePairFrame, type PairNote } from './pairing';
import { mayChange, welcomeRole, welcomeTime, type PersonRole } from './role';
import { noteHubTime } from './hubClock';
import { applyEvent, emptyView, fromSnapshot, num, type View } from './view';

export type LinkState = 'connecting' | 'live' | 'offline';

// The subset of WebSocket the client uses, so tests can stand in for the network.
export interface Socket {
  binaryType: string;
  onopen: (() => void) | null;
  onmessage: ((e: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
  send(data: Uint8Array): void;
  close(): void;
}
export type SocketFactory = (url: string, protocols: string[]) => Socket;

export interface LiveOptions {
  url: string;
  token: string;
  onView: (v: View) => void;
  onState: (s: LinkState) => void;
  // The hub closed the connection twice in a row before saying anything: the session is no longer accepted.
  onRejected?: () => void;
  // A machine refused a command until the owner grants it (ADR-0041): what was asked, so the window can ask the owner.
  onGrantRequired?: (node: string, body: CommandBody) => void;
  // ADR-0065: what the hub tells an owner's window about machines asking to join. `reset` comes first on every new connection (the cards
  // held from the last one are stale until the hub sends the ones waiting now). Memory only: the caller must never store a note.
  onPair?: (n: PairNote) => void;
  // ADR-0106: the role of the person this window signed in as, as the hub's first frame on each connection names it (the hub ends a
  // connection whose role changed); `unspecified` from an older hub, which never says. Called when it changes.
  onRole?: (r: PersonRole) => void;
  socket?: SocketFactory;
  // Waits before reconnect attempt n (0-based); capped exponential with jitter by default (`defaultBackoff`).
  backoffMs?: (attempt: number) => number;
  // Calls `poke` whenever it is a good moment to try the hub again at once; returns how to stop. By default: the browser is
  // back online, the window became visible, or it got the focus.
  wake?: (poke: () => void) => () => void;
}

export type CommandBody = Omit<agora.v1.CommandRequest.$Properties, 'requestId' | 'idempotencyKey' | 'targetNode'>;

export interface Live {
  close(): void;
  // Runs a command on a node through the hub (ADR-0029). Rejects with the error code (`node.offline`, ...).
  command(node: string, body: CommandBody): Promise<agora.v1.CommandOk.$Properties>;
  // A terminal on a node, relayed by the hub. Re-subscribes by itself after a reconnect or when the hub ended the relay.
  terminal(node: string, terminalId: string): TerminalSource;
  // The conversation of an agent that has no terminal (ADR-0038), relayed and resubscribed like a terminal.
  chat(node: string, agentId: string): ChatSource;
  // Try the hub now ("Retry now"): an unreachable hub is dialled at once instead of after the wait; a live link is checked
  // with a ping, so one that went silent (the laptop slept) is noticed within `PONG_WITHIN_MS`.
  retry(): void;
}

export class CommandError extends Error {
  // The error's parameters as text (stable values such as `policy`, never user-facing text).
  constructor(readonly code: string, readonly params: Record<string, string> = {}) {
    super(code);
  }
}

function paramsOf(p: { [k: string]: agora.v1.IParamValue } | null | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(p ?? {})) out[k] = v.str ?? (v.int !== undefined && v.int !== null ? String(v.int) : '');
  return out;
}

// The history a terminal subscription asks for (ADR-0100 decision 2): the window's own setting while its terminal has no screen yet
// (a first open, a reload, a remount: a new stream), 0 once it has one (a reconnect, or after falling behind), where it keeps its own.
export function historyToAsk(t: { history: number; painted: boolean }): number {
  return t.painted ? 0 : Math.max(0, Math.floor(t.history));
}

const browserSocket: SocketFactory = (url, protocols) => new WebSocket(url, protocols) as unknown as Socket;

// A browser cannot see a connection that went silent (a frozen hub, a laptop that slept, a NAT that forgot the connection), so
// the window pings the hub over the protocol's PING/PONG and starts over when nothing at all comes back in time.
export const PING_EVERY_MS = 15_000;
export const PONG_WITHIN_MS = 10_000;
// The longest wait between two attempts while the hub has been away for a few minutes at most (a restart): the window is back
// within about this long of the hub, and a refused dial costs the hub nothing.
export const RETRY_CAP_MS = 1_500;
// After this many attempts (about five minutes of a hub that refuses at once) the hub is probably not restarting, and the waits
// grow to `RETRY_SLOW_CAP_MS`.
export const RETRY_FAST_ATTEMPTS = 200;
export const RETRY_SLOW_CAP_MS = 5_000;
// A connection that has not opened by then is given up and tried again. A hub that froze or slept accepts the TCP connection
// and answers nothing, and the browser may never report an error for it, so without this the window would stay "Connecting"
// long after the hub is back. An open connection is checked by the ping instead (`PING_EVERY_MS`, ADR-0070).
export const OPEN_WITHIN_MS = 8_000;

// Waits before reconnect attempt n (0-based): doubling from 0.5 s up to `RETRY_CAP_MS` (`RETRY_SLOW_CAP_MS` after
// `RETRY_FAST_ATTEMPTS`), each wait shortened by up to a fifth at random so many windows do not all dial a hub that just came back
// in the same instant.
export function defaultBackoff(n: number, random: () => number = Math.random): number {
  const base = Math.min(n < RETRY_FAST_ATTEMPTS ? RETRY_CAP_MS : RETRY_SLOW_CAP_MS, 500 * 2 ** Math.min(n, 16));
  return Math.round(base * (0.8 + 0.2 * random()));
}

const browserWake = (poke: () => void): (() => void) => {
  if (typeof window === 'undefined' || typeof document === 'undefined') return () => {};
  const visible = () => { if (document.visibilityState === 'visible') poke(); };
  window.addEventListener('online', poke);
  window.addEventListener('focus', poke);
  document.addEventListener('visibilitychange', visible);
  return () => {
    window.removeEventListener('online', poke);
    window.removeEventListener('focus', poke);
    document.removeEventListener('visibilitychange', visible);
  };
};

type Due = { due?: number; at?: number; timer?: ReturnType<typeof setTimeout> };

export function connectLive(o: LiveOptions): Live {
  const open = o.socket ?? browserSocket;
  const backoff = o.backoffMs ?? defaultBackoff;
  let view = emptyView();
  let synced = false; // false until a snapshot or a first event proves our position is the hub's
  let attempt = 0;
  let closed = false;
  let sock: Socket | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let isOpen = false;
  let heard = false; // has this connection delivered anything
  let received = 0; // messages on this connection, for the ping's deadline
  let pinger: ReturnType<typeof setInterval> | undefined;
  let opening: ReturnType<typeof setTimeout> | undefined;
  let refusals = 0;
  let requests = 0;
  let role: PersonRole = 'unspecified';
  // ADR-0106: whether this connection has said the role yet (its `Welcome`, or an older hub's first frame). Until then nothing that the
  // role may refuse is sent: a viewer's window must not size or type into a terminal it watches (the hub refuses it and writes it down),
  // and the role may have changed since the last connection. Typing meanwhile waits here (a few frames at most).
  let roleKnown = false;
  let held: (() => void)[] = [];
  const HELD_MAX = 256;
  const settle = (named: PersonRole) => {
    roleKnown = true;
    if (named !== role) { role = named; o.onRole?.(role); }
    const waiting = held;
    held = [];
    if (!mayChange(role)) return;
    for (const t of terminals.values()) if (t.size) sendSize(t.node, t.terminalId, t.size);
    for (const send of waiting) send();
  };
  let nextStream = 3;
  // Bumped for every socket: a retry scheduled on an older one must not resubscribe on the new one (which already did).
  let generation = 0;
  const later = (ms: number, fn: () => void) => {
    const g = generation;
    return setTimeout(() => { if (g === generation && !closed) fn(); }, ms);
  };
  const pending = new Map<string, { resolve: (ok: agora.v1.CommandOk.$Properties) => void; reject: (e: CommandError) => void; node: string; body: CommandBody }>();
  // `size`: what fits the window's view of the terminal, said again after every (re)subscription: the hub counts a window's size
  // only while it watches the terminal, and forgets it with the connection (docs/specs/protocol.md "Shared terminal size").
  // `due`: the connection (generation) on which a resubscription is already scheduled for the stream, when it fires, and its timer.
  // `history`: the lines of history the window asked for (ADR-0100); `painted`: a screen (kind 1) arrived on the stream, so the window
  // has a terminal with history of its own and every later subscription (a reconnect, the hub ending a relay that fell behind) asks
  // for none (`historyToAsk`). `pairing`: the history frames of the current subscription, held for its snapshot; reset whenever a
  // subscription starts or ends, so history never reaches another subscription's snapshot.
  const terminals = new Map<number, { node: string; terminalId: string; onEvent: (e: TerminalEvent) => void; size?: { cols: number; rows: number }; history: number; painted: boolean; pairing: ReturnType<typeof historyPairing> } & Due>();

  const chats = new Map<number, { node: string; agentId: string; onLines: (lines: ChatLine[], replace: boolean) => void } & Due>();

  // At most one resubscription waits per stream and connection: a relay the hub ended while its subscription was still in flight
  // (the node left) is also refused, and the two must not each start a retry that goes on by itself. The hub may send the two in
  // either order, so a request that is due sooner replaces the waiting one; a later one is dropped. The timer holds only the stream
  // id (not the entry and its callbacks), and closing the stream cancels it.
  const resubscribe = (entries: Map<number, Due>, stream: number, ms: number, again: () => void) => {
    const entry = entries.get(stream);
    if (!entry) return;
    const at = performance.now() + ms; // monotonic: a wall clock set back must not delay a retry
    if (entry.due === generation && entry.at !== undefined && entry.at <= at) return;
    clearTimeout(entry.timer);
    entry.due = generation;
    entry.at = at;
    entry.timer = later(ms, () => {
      const e = entries.get(stream);
      if (!e) return;
      e.due = undefined; e.at = undefined; e.timer = undefined;
      again();
    });
  };
  const forget = (entries: Map<number, Due>, stream: number): boolean => {
    const entry = entries.get(stream);
    if (!entry) return false;
    clearTimeout(entry.timer);
    return entries.delete(stream);
  };

  const request = (stream: number, node: string, body: CommandBody): Promise<agora.v1.CommandOk.$Properties> =>
    new Promise((resolve, reject) => {
      if (!isOpen || !sock) { reject(new CommandError('node.offline')); return; }
      const requestId = `c${++requests}`;
      pending.set(requestId, { resolve, reject, node, body });
      const payload = agora.v1.CommandRequest.encode({ ...body, requestId, targetNode: node }).finish();
      sock.send(encodeFrame({ streamId: stream, kind: 0, payload }));
    });

  const subscribeTerminal = (stream: number) => {
    const t = terminals.get(stream);
    if (!t) return;
    t.pairing.reset();
    request(stream, t.node, { terminalSubscribe: { terminalId: t.terminalId, format: '', historyLines: historyToAsk(t) } }).catch(() => {
      // The node may be away or the agent gone; try again a little later while someone is watching.
      resubscribe(terminals, stream, 2000, () => subscribeTerminal(stream));
    });
    // Sent right behind the subscription, so the hub already counts this window as watching when it reads it.
    if (t.size) sendSize(t.node, t.terminalId, t.size);
  };

  const sendSize = (node: string, terminalId: string, size: { cols: number; rows: number }) => {
    // ADR-0106: a viewer's window does not size the agent's terminal (the hub refuses it, and writes the refusal down).
    if (!roleKnown || !mayChange(role)) return;
    request(STREAM_COMMANDS, node, { terminalResize: { terminalId, cols: size.cols, rows: size.rows } }).catch(() => {});
  };

  const subscribeChat = (stream: number) => {
    const c = chats.get(stream);
    if (!c) return;
    request(stream, c.node, { chatSubscribe: { agentId: c.agentId } }).catch(() => {
      resubscribe(chats, stream, 2000, () => subscribeChat(stream));
    });
  };

  const quiet = () => {
    if (pinger) clearInterval(pinger);
    if (opening) clearTimeout(opening);
    pinger = undefined;
    opening = undefined;
  };

  // This connection is over: fail what waits on it and dial again after the backoff. `mayBeRefusal` is false when the window gave
  // up on it itself (silent, or never opened), which says nothing about the session.
  const lost = (s: Socket, mayBeRefusal: boolean) => {
    if (sock !== s) return; // already given up
    sock = undefined;
    quiet();
    // Only a connection that opened and was dropped without a word counts: an unreachable hub is not a refusal.
    const dropped = mayBeRefusal && isOpen && !heard;
    isOpen = false;
    refusals = heard ? 0 : dropped ? refusals + 1 : refusals;
    for (const p of pending.values()) p.reject(new CommandError('node.offline'));
    // History whose snapshot never came belongs to a subscription that is over.
    for (const t of terminals.values()) t.pairing.reset();
    pending.clear();
    if (refusals >= 2 && !closed) { closed = true; unwake(); o.onRejected?.(); return; }
    if (closed) return;
    o.onState('offline');
    if (timer) clearTimeout(timer);
    timer = setTimeout(start, backoff(attempt++));
  };

  // Gives up on a connection that the browser still calls open (or still opening) but that is silent.
  const abandon = (s: Socket) => {
    lost(s, false);
    s.onopen = null;
    s.onmessage = null;
    s.onclose = null;
    try { s.close(); } catch { /* already closed */ }
  };

  // Pings the hub; when nothing at all arrives within `PONG_WITHIN_MS`, the connection is dead.
  const probe = (s: Socket) => {
    if (sock !== s || !isOpen) return;
    const mark = received;
    s.send(encodeFrame({ streamId: STREAM_CONTROL, kind: CONTROL_KIND.PING, payload: new Uint8Array() }));
    setTimeout(() => { if (sock === s && received === mark) abandon(s); }, PONG_WITHIN_MS);
  };

  const start = () => {
    timer = undefined;
    o.onState('connecting');
    generation++;
    const s = open(o.url, ['agora.v1', `agora.bearer.${o.token}`]);
    sock = s;
    s.binaryType = 'arraybuffer';
    // A connection that has not opened within OPEN_WITHIN_MS is given up by the window itself: the browser's own close may
    // come much later, or never.
    opening = setTimeout(() => { if (sock === s && !isOpen) abandon(s); }, OPEN_WITHIN_MS);
    s.onopen = () => {
      if (closed || sock !== s) return;
      if (opening) clearTimeout(opening);
      opening = undefined;
      isOpen = true;
      roleKnown = false;
      held = [];
      heard = false;
      o.onPair?.({ type: 'reset' });
      quiet();
      pinger = setInterval(() => probe(s), PING_EVERY_MS);
      const sub = agora.v1.Subscribe.encode(synced ? { afterPos: view.pos } : {}).finish();
      s.send(encodeFrame({ streamId: STREAM_EVENTS, kind: EVENT_KIND.SUBSCRIBE, payload: sub }));
      // The hub answers a subscription with nothing when nothing happened since our position (a hub that restarted with no
      // machine connected yet), so the window would say "Connecting" until the next event. The hub sends what the subscription
      // owes before it reads the next frame, so its answer to this ping says the window is up to date: live.
      probe(s);
      for (const stream of terminals.keys()) subscribeTerminal(stream);
      for (const stream of chats.keys()) subscribeChat(stream);
    };
    s.onmessage = (m) => {
      if (closed || sock !== s) return;
      // Any delivered message proves this was not a silent authentication refusal, even when its shape is a protocol error.
      heard = true;
      received++;
      // One WebSocket message is one binary protocol frame. Text (and any other browser value) is a protocol error, just like a
      // malformed binary envelope: close so the next connection starts from the last proven event position.
      if (!(m.data instanceof ArrayBuffer || ArrayBuffer.isView(m.data))) { s.close(); return; }
      const bytes = m.data instanceof ArrayBuffer ? new Uint8Array(m.data) : new Uint8Array(m.data.buffer, m.data.byteOffset, m.data.byteLength);
      const f = decodeFrame(bytes);
      if (!f) { s.close(); return; }
      // ADR-0106: the first frame says the role: a `Welcome`, or anything else from an older hub, which names none.
      const welcome = f.streamId === STREAM_CONTROL && f.kind === CONTROL_KIND.WELCOME;
      if (!roleKnown && !welcome) settle('unspecified');
      if (f.streamId === STREAM_COMMANDS && f.kind === 1) {
        try {
          const r = agora.v1.CommandResponse.decode(f.payload);
          const p = pending.get(r.requestId);
          pending.delete(r.requestId);
          if (r.err) {
            if (p && r.err.code === 'approval.grant_required') o.onGrantRequired?.(p.node, p.body);
            p?.reject(new CommandError(r.err.code ?? 'error', paramsOf(r.err.params)));
          }
          else p?.resolve(r.ok ?? agora.v1.CommandOk.create());
        } catch { /* a garbled answer: the request stays pending until the connection ends */ }
        return;
      }
      if (f.streamId >= 3 && chats.has(f.streamId)) {
        const c = chats.get(f.streamId);
        if (!c) return;
        if (f.kind === STREAM_CLOSE) { const id = f.streamId; resubscribe(chats, id, 500, () => subscribeChat(id)); return; }
        try {
          if (f.kind === CHAT_KIND.SNAPSHOT) c.onLines((agora.v1.ChatSnapshot.decode(f.payload).entries ?? []).map(chatLine), true);
          else if (f.kind === CHAT_KIND.ENTRY) c.onLines([chatLine(agora.v1.ChatEntry.decode(f.payload))], false);
        } catch { /* a garbled frame: the next one carries on */ }
        return;
      }
      if (f.streamId >= 3) {
        const t = terminals.get(f.streamId);
        if (!t) return;
        if (f.kind === STREAM_CLOSE) {
          // The hub ended the relay (slow client, node left, exit): ask again for a fresh snapshot. History of this relay whose
          // snapshot never came is dropped.
          t.pairing.reset();
          const id = f.streamId;
          resubscribe(terminals, id, 500, () => subscribeTerminal(id));
          return;
        }
        const d = decodeTerminalFrame(f.kind, f.payload);
        const e = d && t.pairing.frame(d);
        if (e?.type === 'snapshot') t.painted = true;
        if (e) t.onEvent(e);
        return;
      }
      if (f.streamId === STREAM_CONTROL && f.kind === CONTROL_KIND.WELCOME) {
        noteHubTime(welcomeTime(f.payload));
        settle(welcomeRole(f.payload));
        return;
      }
      if (f.streamId === STREAM_CONTROL && f.kind === CONTROL_KIND.PONG && synced) {
        attempt = 0;
        o.onState('live');
        return;
      }
      if (f.streamId !== STREAM_EVENTS) return;
      if (f.kind === EVENT_KIND.PAIR_REQUESTED || f.kind === EVENT_KIND.PAIR_DECIDED) {
        const n = decodePairFrame(f.kind, f.payload, Date.now());
        if (n) o.onPair?.(n);
        return;
      }
      try {
        if (f.kind === EVENT_KIND.SNAPSHOT) {
          view = fromSnapshot(agora.v1.Snapshot.decode(f.payload));
          synced = true;
        } else if (f.kind === EVENT_KIND.POSITIONED_EVENT) {
          const pe = agora.v1.PositionedEvent.decode(f.payload);
          if (pe.event) applyEvent(view, num(pe.pos), pe.event);
          synced = true;
        } else {
          return; // unknown kinds are ignored: a newer hub may add some
        }
      } catch {
        s.close(); // undecodable payload: start over from a fresh snapshot
        synced = false;
        return;
      }
      attempt = 0;
      o.onState('live');
      o.onView(view);
    };
    s.onclose = () => lost(s, true);
  };

  // Try now: dial an unreachable hub at once, or check that a live link still answers. A dial already under way is left to
  // finish (or to time out), so a burst of focus events does not start it over and over.
  const retry = () => {
    if (closed) return;
    if (sock) { if (isOpen) probe(sock); return; }
    if (timer) clearTimeout(timer);
    attempt = 0;
    start();
  };
  const unwake = (o.wake ?? browserWake)(retry);

  start();
  return {
    command: (node, body) => request(STREAM_COMMANDS, node, body),
    terminal: (node, terminalId) => ({
      connect(onEvent, options) {
        const stream = nextStream++;
        terminals.set(stream, { node, terminalId, onEvent, history: options?.historyLines ?? 0, painted: false, pairing: historyPairing() });
        if (isOpen) subscribeTerminal(stream);
        return {
          // A closed session is inert: a late keystroke or resize (a frame callback after the pane went) is not sent.
          sendInput(bytes) {
            if (!terminals.has(stream)) return;
            const send = () => { if (terminals.has(stream)) request(STREAM_COMMANDS, node, { terminalInput: { terminalId, data: bytes } }).catch(() => {}); };
            // ADR-0106: a viewer's keys are not sent (the hub would refuse each and write it down); before the role is known they wait.
            if (!roleKnown) { if (held.length < HELD_MAX) held.push(send); return; }
            if (mayChange(role)) send();
          },
          resize(cols, rows) {
            const t = terminals.get(stream);
            if (!t) return;
            t.size = { cols, rows };
            if (isOpen) sendSize(node, terminalId, { cols, rows });
          },
          close() {
            if (!forget(terminals, stream)) return;
            if (isOpen && sock) sock.send(encodeFrame({ streamId: stream, kind: STREAM_CLOSE, payload: new Uint8Array() }));
          },
        };
      },
    }),
    chat: (node, agentId) => ({
      connect(onLines) {
        const stream = nextStream++;
        chats.set(stream, { node, agentId, onLines });
        if (isOpen) subscribeChat(stream);
        return {
          close() {
            if (!forget(chats, stream)) return;
            if (isOpen && sock) sock.send(encodeFrame({ streamId: stream, kind: STREAM_CLOSE, payload: new Uint8Array() }));
          },
        };
      },
    }),
    retry,
    close() {
      closed = true;
      unwake();
      quiet();
      if (timer) clearTimeout(timer);
      if (opening) clearTimeout(opening);
      const s = sock;
      if (s) {
        lost(s, false);
        s.close();
      }
    },
  };
}
