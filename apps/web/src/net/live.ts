// The live connection to the hub (ADR-0027): subscribe, take the snapshot, follow positioned events, and come back
// by itself with the position it has. A browser cannot set headers on a WebSocket, so the token travels as a
// subprotocol (`agora.v1`, `agora.bearer.<token>`).

import { agora } from '../gen/agora.js';
import { decodeFrame, encodeFrame, EVENT_KIND, STREAM_EVENTS } from './frame';
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
  socket?: SocketFactory;
  // Waits before reconnect attempt n (0-based); capped exponential by default.
  backoffMs?: (attempt: number) => number;
}

export interface Live { close(): void }

const browserSocket: SocketFactory = (url, protocols) => new WebSocket(url, protocols) as unknown as Socket;
const defaultBackoff = (n: number) => Math.min(30_000, 500 * 2 ** n);

export function connectLive(o: LiveOptions): Live {
  const open = o.socket ?? browserSocket;
  const backoff = o.backoffMs ?? defaultBackoff;
  let view = emptyView();
  let synced = false; // false until a snapshot or a first event proves our position is the hub's
  let attempt = 0;
  let closed = false;
  let sock: Socket | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const start = () => {
    o.onState('connecting');
    const s = open(o.url, ['agora.v1', `agora.bearer.${o.token}`]);
    sock = s;
    s.binaryType = 'arraybuffer';
    s.onopen = () => {
      const sub = agora.v1.Subscribe.encode(synced ? { afterPos: view.pos } : {}).finish();
      s.send(encodeFrame({ streamId: STREAM_EVENTS, kind: EVENT_KIND.SUBSCRIBE, payload: sub }));
    };
    s.onmessage = (m) => {
      if (!(m.data instanceof ArrayBuffer || ArrayBuffer.isView(m.data))) return;
      const bytes = m.data instanceof ArrayBuffer ? new Uint8Array(m.data) : new Uint8Array((m.data as ArrayBufferView).buffer);
      const f = decodeFrame(bytes);
      if (!f) { s.close(); return; }
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
    s.onclose = () => {
      if (closed) return;
      o.onState('offline');
      timer = setTimeout(start, backoff(attempt++));
    };
  };

  start();
  return {
    close() {
      closed = true;
      if (timer) clearTimeout(timer);
      sock?.close();
    },
  };
}
