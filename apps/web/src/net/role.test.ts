// ADR-0106: the role the hub's first frame names, and what it tells the window without asking.
import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { CONTROL_KIND, decodeFrame, encodeFrame, EVENT_KIND, STREAM_CONTROL, STREAM_COMMANDS, STREAM_EVENTS } from './frame';
import { connectLive, type Socket } from './live';
import { mayChange, ownerOfRole, welcomeRole, type PersonRole } from './role';

const v1 = agora.v1;
const welcome = (r: agora.v1.PersonRole) => v1.Welcome.encode({ chosenGeneration: 1, personRole: r }).finish();

describe('the role a Welcome names', () => {
  it('reads each role, and none from a node\'s Welcome or a garbled one', () => {
    expect(welcomeRole(welcome(v1.PersonRole.PERSON_ROLE_OWNER))).toBe('owner');
    expect(welcomeRole(welcome(v1.PersonRole.PERSON_ROLE_MEMBER))).toBe('member');
    expect(welcomeRole(welcome(v1.PersonRole.PERSON_ROLE_VIEWER))).toBe('viewer');
    expect(welcomeRole(v1.Welcome.encode({ chosenGeneration: 1, peerId: 'n1' }).finish())).toBe('unspecified');
    expect(welcomeRole(new Uint8Array([0xff, 0xff, 0xff]))).toBe('unspecified');
  });

  it('a member or viewer is not the owner without asking; the owner and an older hub ask', () => {
    expect(ownerOfRole('member')).toBe('no');
    expect(ownerOfRole('viewer')).toBe('no');
    expect(ownerOfRole('owner')).toBeUndefined();
    expect(ownerOfRole('unspecified')).toBeUndefined();
  });

  it('only a viewer\'s window offers nothing that changes something', () => {
    expect((['unspecified', 'owner', 'member', 'viewer'] as const).map(mayChange)).toEqual([true, true, true, false]);
  });
});

class FakeSocket implements Socket {
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  send(d: Uint8Array) { this.sent.push(d); }
  close() { this.onclose?.(); }
  deliver(streamId: number, kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
}

const terminalCommands = (s: FakeSocket) => s.sent.map((b) => decodeFrame(b)).filter((f) => f && f.kind === 0 && (f.streamId === STREAM_COMMANDS || f.streamId >= 3))
  .map((f) => v1.CommandRequest.decode(f?.payload as Uint8Array)).map((c) => c.terminalSubscribe ? 'subscribe' : c.terminalResize ? 'resize' : c.terminalInput ? 'input' : '?');

describe('the live connection and the role (ADR-0106)', () => {
  const connect = () => {
    const sockets: FakeSocket[] = [];
    const roles: PersonRole[] = [];
    const live = connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: () => { const s = new FakeSocket(); sockets.push(s); return s; },
      onState: () => {}, onView: () => {}, onRole: (r) => roles.push(r),
    });
    return { live, sockets, roles };
  };
  const snapshot = v1.Snapshot.encode({ pos: 1 }).finish();

  it('takes the role from the hub\'s first frame, and an older hub\'s first frame says unspecified', async () => {
    const { live, sockets, roles } = connect();
    const first = sockets[0] as FakeSocket;
    first.onopen?.();
    first.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, welcome(v1.PersonRole.PERSON_ROLE_MEMBER));
    first.deliver(STREAM_EVENTS, EVENT_KIND.SNAPSHOT, snapshot);
    expect(roles).toEqual(['member']);
    // Made the owner: the hub ends the connection, and the next one says so.
    first.onclose?.();
    await new Promise((r) => setTimeout(r, 5));
    const second = sockets[1] as FakeSocket;
    second.onopen?.();
    second.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, welcome(v1.PersonRole.PERSON_ROLE_OWNER));
    expect(roles).toEqual(['member', 'owner']);
    // An older hub (a downgrade): its first frame is the snapshot.
    second.onclose?.();
    await new Promise((r) => setTimeout(r, 5));
    const third = sockets[2] as FakeSocket;
    third.onopen?.();
    third.deliver(STREAM_EVENTS, EVENT_KIND.SNAPSHOT, snapshot);
    expect(roles).toEqual(['member', 'owner', 'unspecified']);
    live.close();
  });

  it('a viewer\'s window watches a terminal without typing into it or sizing it, also after a lost link', async () => {
    const { live, sockets } = connect();
    const first = sockets[0] as FakeSocket;
    first.onopen?.();
    first.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, welcome(v1.PersonRole.PERSON_ROLE_VIEWER));
    first.deliver(STREAM_EVENTS, EVENT_KIND.SNAPSHOT, snapshot);
    const session = live.terminal('laptop', 't-1').connect(() => {});
    session.resize(120, 40);
    session.sendInput(new TextEncoder().encode('ls\r'));
    expect(terminalCommands(first)).toEqual(['subscribe']);
    // The terminal is asked for again as soon as the next connection opens, before its Welcome: still no size.
    first.onclose?.();
    await new Promise((r) => setTimeout(r, 5));
    const second = sockets[1] as FakeSocket;
    second.onopen?.();
    expect(terminalCommands(second)).toEqual(['subscribe']);
    live.close();
  });

  it('a viewer made a member sizes the terminals it watches once the next connection names the new role', async () => {
    const { live, sockets } = connect();
    const first = sockets[0] as FakeSocket;
    first.onopen?.();
    first.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, welcome(v1.PersonRole.PERSON_ROLE_VIEWER));
    const session = live.terminal('laptop', 't-1').connect(() => {});
    session.resize(120, 40);
    first.onclose?.();
    await new Promise((r) => setTimeout(r, 5));
    const second = sockets[1] as FakeSocket;
    second.onopen?.();
    expect(terminalCommands(second)).toEqual(['subscribe']);
    second.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, welcome(v1.PersonRole.PERSON_ROLE_MEMBER));
    expect(terminalCommands(second)).toEqual(['subscribe', 'resize']);
    live.close();
  });

  it('a member\'s window types and sizes as before', () => {
    const { live, sockets } = connect();
    const first = sockets[0] as FakeSocket;
    first.onopen?.();
    first.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, welcome(v1.PersonRole.PERSON_ROLE_MEMBER));
    const session = live.terminal('laptop', 't-1').connect(() => {});
    session.resize(120, 40);
    session.sendInput(new TextEncoder().encode('ls\r'));
    expect(terminalCommands(first)).toEqual(['subscribe', 'resize', 'input']);
    live.close();
  });
});
