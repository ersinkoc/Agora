import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { connectLive, type Socket } from './live';
import { decodeFrame, encodeFrame, EVENT_KIND, STREAM_EVENTS } from './frame';
import type { View } from './view';
class TestSocket implements Socket {
 binaryType = '';
 onopen: (() => void) | null = null;
 onmessage: ((e: {data:unknown}) => void) | null = null;
 onclose: (() => void) | null = null;
 sent: Uint8Array[] = [];
 send(data: Uint8Array) { this.sent.push(data); }
 close() { this.onclose?.(); }
 snapshot(payload: Uint8Array) { const b=encodeFrame({streamId:STREAM_EVENTS,kind:EVENT_KIND.SNAPSHOT,payload});this.onmessage?.({data:b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength)}); }
}
it('the existing live connection recovers a late window from the hub forced reconnect and real snapshots', async () => {
 const hidden = readFileSync('/tmp/agora-535-fixed-wire/hidden.snapshot.bin');
 const recovered = readFileSync('/tmp/agora-535-fixed-wire/recovered.snapshot.bin');
 const oldPos = Number(agora.v1.Snapshot.decode(hidden).pos);
 const sockets:TestSocket[]=[];
 let current:View|undefined;
 let rejected=false;
 const states:string[]=[];
 vi.useFakeTimers();
 const live=connectLive({url:'ws://test.invalid',token:'test-only',onView:v=>{current=v;},onState:s=>states.push(s),onRejected:()=>{rejected=true;},socket:()=>{const s=new TestSocket();sockets.push(s);return s;},backoffMs:()=>1,wake:()=>()=>{}});
 try {
  sockets[0]!.onopen?.();sockets[0]!.snapshot(hidden);
  expect(current?.agents.size).toBe(0);expect(current?.workspaces.size).toBe(0);
  sockets[0]!.onclose?.();
  await vi.advanceTimersByTimeAsync(1);
  expect(sockets).toHaveLength(2);
  sockets[1]!.onopen?.();
  const frame=sockets[1]!.sent.map(decodeFrame).find(f=>f?.streamId===STREAM_EVENTS&&f.kind===EVENT_KIND.SUBSCRIBE)!;
  expect(Number(agora.v1.Subscribe.decode(frame.payload).afterPos)).toBe(oldPos);
  // The server proof produced this exact snapshot for that resume position.
  sockets[1]!.snapshot(recovered);
  expect(current?.nodes.get('n1')?.hostname).toBe('laptop');
  expect(current?.agents.size).toBe(1);expect(current?.agents.get('n1/a')?.running).toBe(true);
  expect(current?.checkouts.size).toBe(1);expect(current?.workspaces.size).toBe(1);expect(current?.agentWorkspaces.size).toBe(1);
  expect(states).toEqual(['connecting','live','offline','connecting','live']);expect(rejected).toBe(false);
  console.info(JSON.stringify({oldPos,returnedPos:current?.pos,states,hostname:current?.nodes.get('n1')?.hostname,agents:current?.agents.size,workspaces:current?.workspaces.size}));
 } finally {live.close();vi.useRealTimers();}
});
