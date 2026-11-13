// The live connection to the hub (ADR-0027): subscribe, take the snapshot, follow positioned events, and come back
// by itself with the position it has. A browser cannot set headers on a WebSocket, so the token travels as a
// subprotocol (`agora.v1`, `agora.bearer.<token>`).

import { agora } from '../gen/agora.js';
import { chatLine, type ChatLine, type ChatSource } from './chat';
const CHAT_KIND = { SNAPSHOT: 10, ENTRY: 11 } as const;
import { decodeTerminalFrame, type TerminalEvent } from '../terminal/frames';
import type { TerminalSource } from '../terminal/source';
import { CONTROL_KIND, decodeFrame, encodeFrame, EVENT_KIND, STREAM_COMMANDS, STREAM_CLOSE, STREAM_CONTROL, STREAM_EVENTS } from './frame';
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
  constructor(readonly code: string) {
    super(code);
  }
}

const browserSocket: SocketFactory = (url, protocols) => new WebSocket(url, protocols) as unknown as Socket;

// A browser cannot see a connection that went silent (a frozen hub, a laptop that slept, a NAT that forgot the connection), so
// the window pings the hub over the protocol's PING/PONG and starts over when nothing at all comes back in time.
export const PING_EVERY_MS = 15_000;
export const PONG_WITHIN_MS = 10_000;
// The longest wait between two attempts: the window is back within a few seconds of the hub.
export const RETRY_CAP_MS = 5_000;

// Waits before reconnect attempt n (0-based): doubling from 0.5 s up to `RETRY_CAP_MS`, each wait shortened by up to a fifth at
// random so many windows do not all dial a hub that just came back in the same instant.
export function defaultBackoff(n: number, random: () => number = Math.random): number {
  const base = Math.min(RETRY_CAP_MS, 500 * 2 ** Math.min(n, 16));
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
  let nextStream = 3;
  // Bumped for every socket: a retry scheduled on an older one must not resubscribe on the new one (which already did).
  let generation = 0;
  const later = (ms: number, fn: () => void) => {
    const g = generation;
    setTimeout(() => { if (g === generation && !closed) fn(); }, ms);
  };
  const pending = new Map<string, { resolve: (ok: agora.v1.CommandOk.$Properties) => void; reject: (e: CommandError) => void; node: string; body: CommandBody }>();
  const terminals = new Map<number, { node: string; terminalId: string; onEvent: (e: TerminalEvent) => void }>();

  const chats = new Map<number, { node: string; agentId: string; onLines: (lines: ChatLine[], replace: boolean) => void }>();

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
    request(stream, t.node, { terminalSubscribe: { terminalId: t.terminalId, format: '' } }).catch(() => {
      // The node may be away or the agent gone; try again a little later while someone is watching.
      if (terminals.has(stream)) later(2000, () => subscribeTerminal(stream));
    });
  };

  const subscribeChat = (stream: number) => {
    const c = chats.get(stream);
    if (!c) return;
    request(stream, c.node, { chatSubscribe: { agentId: c.agentId } }).catch(() => {
      if (chats.has(stream)) later(2000, () => subscribeChat(stream));
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
    if (refusals >= 2 && !closed) { closed = true; o.onRejected?.(); return; }
    for (const p of pending.values()) p.reject(new CommandError('node.offline'));
    pending.clear();
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
    // A dial that does not open in time (a host that went away mid-handshake) is given up too.
    opening = setTimeout(() => { if (sock === s && !isOpen) abandon(s); }, PING_EVERY_MS);
    s.onopen = () => {
      isOpen = true;
      heard = false;
      quiet();
      pinger = setInterval(() => probe(s), PING_EVERY_MS);
      const sub = agora.v1.Subscribe.encode(synced ? { afterPos: view.pos } : {}).finish();
      s.send(encodeFrame({ streamId: STREAM_EVENTS, kind: EVENT_KIND.SUBSCRIBE, payload: sub }));
      for (const stream of terminals.keys()) subscribeTerminal(stream);
      for (const stream of chats.keys()) subscribeChat(stream);
    };
    s.onmessage = (m) => {
      if (!(m.data instanceof ArrayBuffer || ArrayBuffer.isView(m.data))) return;
      const bytes = m.data instanceof ArrayBuffer ? new Uint8Array(m.data) : new Uint8Array((m.data as ArrayBufferView).buffer);
      heard = true;
      received++;
      const f = decodeFrame(bytes);
      if (!f) { s.close(); return; }
      if (f.streamId === STREAM_COMMANDS && f.kind === 1) {
        try {
          const r = agora.v1.CommandResponse.decode(f.payload);
          const p = pending.get(r.requestId);
          pending.delete(r.requestId);
          if (r.err) {
            if (p && r.err.code === 'approval.grant_required') o.onGrantRequired?.(p.node, p.body);
            p?.reject(new CommandError(r.err.code ?? 'error'));
          }
          else p?.resolve(r.ok ?? agora.v1.CommandOk.create());
        } catch { /* a garbled answer: the request stays pending until the connection ends */ }
        return;
      }
      if (f.streamId >= 3 && chats.has(f.streamId)) {
        const c = chats.get(f.streamId);
        if (!c) return;
        if (f.kind === STREAM_CLOSE) { later(500, () => subscribeChat(f.streamId)); return; }
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
          // The hub ended the relay (slow client, node left, exit): ask again for a fresh snapshot.
          later(500, () => subscribeTerminal(f.streamId));
          return;
        }
        const e = decodeTerminalFrame(f.kind, f.payload);
        if (e) t.onEvent(e);
        return;
      }
      if (f.streamId !== STREAM_EVENTS) return;
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
      connect(onEvent) {
        const stream = nextStream++;
        terminals.set(stream, { node, terminalId, onEvent });
        if (isOpen) subscribeTerminal(stream);
        return {
          sendInput(bytes) {
            request(STREAM_COMMANDS, node, { terminalInput: { terminalId, data: bytes } }).catch(() => {});
          },
          resize(cols, rows) {
            request(STREAM_COMMANDS, node, { terminalResize: { terminalId, cols, rows } }).catch(() => {});
          },
          close() {
            terminals.delete(stream);
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
            chats.delete(stream);
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
      sock?.close();
    },
  };
}
