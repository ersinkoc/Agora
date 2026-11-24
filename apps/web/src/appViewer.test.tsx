// @vitest-environment jsdom
// ADR-0106: the whole window, connected to a hub (a stand-in WebSocket) whose first frame names the person's role. A viewer's window offers
// nothing that starts or changes something; a member's does.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from './gen/agora.js';
import { App } from './App';
import { CONTROL_KIND, decodeFrame, encodeFrame, EVENT_KIND, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS } from './net/frame';
import { setLanguage, t } from './i18n';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const v1 = agora.v1;

class HubSocket {
  static last: HubSocket | undefined;
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  constructor(readonly url: string, readonly protocols: string[]) { HubSocket.last = this; }
  send(d: Uint8Array) { this.sent.push(d); }
  close() {}
  deliver(streamId: number, kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
}

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  window.localStorage.setItem('agora.hubToken', 'tok');
  vi.stubGlobal('WebSocket', HubSocket);
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })));
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); window.localStorage.clear(); });

async function windowAs(role: agora.v1.PersonRole): Promise<string> {
  await act(async () => { root.render(<App />); });
  const s = HubSocket.last as HubSocket;
  await act(async () => {
    s.onopen?.();
    s.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, v1.Welcome.encode({ chosenGeneration: 1, personRole: role }).finish());
    s.deliver(STREAM_EVENTS, EVENT_KIND.SNAPSHOT, v1.Snapshot.encode({ pos: 3, nodes: [{ nodeId: 'n1', hostname: 'Box', online: true }] }).finish());
  });
  return host.innerHTML;
}

/** The commands this window sent the hub, by their schema field name (the oneof's name). */
const asked = () => (HubSocket.last?.sent ?? []).map((b) => decodeFrame(b)).filter((f) => f && f.streamId === STREAM_COMMANDS && f.kind === 0)
  .map((f) => v1.CommandRequest.decode(f?.payload as Uint8Array)).map((r) => (r.pairStatus ? 'pairStatus' : r.usageReport ? 'usageReport' : r.command ?? 'other'));

describe('the window and the person\'s role (ADR-0106)', () => {
  it('a member\'s window offers starting an agent, and asks the hub neither whose window it is nor the cost', async () => {
    const html = await windowAs(v1.PersonRole.PERSON_ROLE_MEMBER);
    expect(html).toContain(t('action-start-agent'));
    expect(html).toContain(t('fleet-firstrun-title'));
    expect(asked()).not.toContain('pairStatus');
    expect(asked()).not.toContain('usageReport');
  });
  it('a viewer\'s window offers nothing to start, says what they will see, and asks nothing the hub would refuse', async () => {
    const html = await windowAs(v1.PersonRole.PERSON_ROLE_VIEWER);
    expect(html).not.toContain(t('action-start-agent'));
    expect(html).not.toContain(t('fleet-firstrun-title'));
    expect(html).toContain(t('fleet-watch-title'));
    expect(html).toContain(t('fleet-watch-empty'));
    expect(asked()).not.toContain('pairStatus');
    expect(asked()).not.toContain('usageReport');
  });
  it('the owner\'s window asks whose window it is (to learn whether pairing is on)', async () => {
    const html = await windowAs(v1.PersonRole.PERSON_ROLE_OWNER);
    expect(html).toContain(t('action-start-agent'));
    expect(asked()).toContain('pairStatus');
  });
});
