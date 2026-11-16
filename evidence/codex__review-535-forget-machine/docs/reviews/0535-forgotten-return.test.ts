import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { applyEvent, fromSnapshot } from './view';
const wire = '/tmp/agora-535-wire/';
const hidden = agora.v1.Snapshot.decode(readFileSync(wire + 'hidden.snapshot.bin'));
const current = agora.v1.Snapshot.decode(readFileSync(wire + 'returned.snapshot.bin'));
const event = agora.v1.Event.decode(readFileSync(wire + 'live.event.bin'));
const pos = Number(readFileSync(wire + 'position.txt', 'utf8'));
it('a fresh snapshot after return contains the retained machine, folder and running agent', () => {
 const view = fromSnapshot(current);
 expect(view.nodes.get('n')?.hostname).toBe('laptop');
 expect(view.agents.size).toBe(1);
 expect(view.checkouts.size).toBe(1);
 expect(view.workspaces.size).toBe(1);
});
it('a window opened while forgotten recovers the same data from the actual return event', () => {
 const view = fromSnapshot(hidden);
 applyEvent(view, pos, event);
 const expected = fromSnapshot(current);
 const got = {hostname:view.nodes.get('n')?.hostname,agents:view.agents.size,checkouts:view.checkouts.size,workspaces:view.workspaces.size};
 const want = {hostname:expected.nodes.get('n')?.hostname,agents:expected.agents.size,checkouts:expected.checkouts.size,workspaces:expected.workspaces.size};
 console.info(JSON.stringify({receivedEvent:event.body,got,want}));
 expect(got).toEqual(want);
});
