import { describe, expect, it } from 'vitest';
import type { TerminalSource } from '../terminal/source';
import { type Action, type Workspace, reduce, workspaceOf } from './engine';
import { TerminalStreams, assignAttention, streamPlan, type Bindings } from './streams';
import { bindOf, defaultLayout, panesOf } from './tree';

// Fake sources that count what is connected, per agent.
function fakes() {
  const live = new Map<string, number>();
  let connects = 0;
  const sourceFor = (agent: string): TerminalSource => ({
    connect() {
      connects++;
      live.set(agent, (live.get(agent) ?? 0) + 1);
      return { sendInput() {}, resize() {}, close() { live.set(agent, (live.get(agent) ?? 1) - 1); } };
    },
  });
  return { sourceFor, live: () => [...live].filter(([, n]) => n > 0).map(([a]) => a).sort(), connects: () => connects };
}
const none: Bindings = { attention: new Map() };
const terminals = (a: string) => !a.startsWith('chat');
const idOf = (ws: Workspace, agent: string) => panesOf(ws.layout.root).find((p) => bindOf(p) === agent)!.id;

describe('only panes on screen hold a terminal stream', () => {
  it('a 2 × 2 grid subscribes four; a background tab none; closing the grid closes them all', () => {
    const f = fakes();
    const streams = new TerminalStreams(f.sourceFor);
    let ws = reduce(workspaceOf(defaultLayout()), { type: 'preset', preset: '2x2', agents: ['a', 'b', 'c', 'd', 'e'] });
    streams.sync(streamPlan(ws, false, none, terminals));
    expect(f.live()).toEqual(['a', 'b', 'c', 'd']); // e is a tab behind d
    ws = reduce(ws, { type: 'activate', pane: idOf(ws, 'e') });
    streams.sync(streamPlan(ws, false, none, terminals));
    expect(f.live()).toEqual(['a', 'b', 'c', 'e']);
    streams.close();
    expect(f.live()).toEqual([]);
  });
  it('maximizing one unsubscribes the others; restoring subscribes them again', () => {
    const f = fakes();
    const streams = new TerminalStreams(f.sourceFor);
    let ws = reduce(workspaceOf(defaultLayout()), { type: 'preset', preset: '2x2', agents: ['a', 'b', 'c', 'd'] });
    streams.sync(streamPlan(ws, false, none, terminals));
    ws = reduce(ws, { type: 'maximize', pane: idOf(ws, 'b') });
    streams.sync(streamPlan(ws, false, none, terminals));
    expect(f.live()).toEqual(['b']);
    expect(f.connects()).toBe(4); // b kept its stream
    ws = reduce(ws, { type: 'restore' });
    streams.sync(streamPlan(ws, false, none, terminals));
    expect(f.live()).toEqual(['a', 'b', 'c', 'd']);
  });
  it('resizing, swapping and moving keep every stream (nothing reconnects)', () => {
    const f = fakes();
    const streams = new TerminalStreams(f.sourceFor);
    let ws = reduce(workspaceOf(defaultLayout()), { type: 'preset', preset: '2x2', agents: ['a', 'b', 'c', 'd'] });
    streams.sync(streamPlan(ws, false, none, terminals));
    for (const step of [
      { type: 'resize', path: [0], index: 0, delta: 0.15 },
      { type: 'resize', path: [], index: 0, delta: -0.2 },
      { type: 'swap', a: idOf(ws, 'a'), b: idOf(ws, 'd') },
      { type: 'move', pane: idOf(ws, 'c'), target: idOf(ws, 'b'), place: 'bottom' },
      { type: 'split', pane: idOf(ws, 'a'), direction: 'row' },
      { type: 'preset', preset: '3x2', agents: ['a', 'b', 'c', 'd'] },
    ] as Action[]) {
      ws = reduce(ws, step);
      streams.sync(streamPlan(ws, false, none, terminals));
    }
    expect(f.live()).toEqual(['a', 'b', 'c', 'd']);
    expect(f.connects()).toBe(4);
  });
  it('a phone shows one pane, so one stream', () => {
    const f = fakes();
    const streams = new TerminalStreams(f.sourceFor);
    const ws = reduce(workspaceOf(defaultLayout()), { type: 'preset', preset: '2x2', agents: ['a', 'b', 'c', 'd'] });
    streams.sync(streamPlan(reduce(ws, { type: 'focus', pane: idOf(ws, 'c') }), true, none, terminals));
    expect(f.live()).toEqual(['c']);
  });
  it('chat agents and other panels hold no terminal stream', () => {
    let ws = reduce(workspaceOf(defaultLayout()), { type: 'preset', preset: '2x2', agents: ['a', 'chat-1'] });
    const [, , third] = panesOf(ws.layout.root);
    ws = reduce(ws, { type: 'bind', pane: third!.id, panel: 'approvals' });
    expect([...streamPlan(ws, false, none, terminals).values()]).toEqual(['a']);
  });
  it('a pane that follows attention moves its stream with the agent it shows', () => {
    const f = fakes();
    const streams = new TerminalStreams(f.sourceFor);
    let ws = workspaceOf(defaultLayout());
    ws = reduce(ws, { type: 'bind', pane: ws.focus!, bind: { follow: 'attention' } });
    const pane = ws.focus!;
    let attention = assignAttention([pane], new Map(), ['x', 'y']);
    streams.sync(streamPlan(ws, false, { attention }, terminals));
    expect(f.live()).toEqual(['x']);
    attention = assignAttention([pane], attention, ['y']);
    streams.sync(streamPlan(ws, false, { attention }, terminals));
    expect(f.live()).toEqual(['y']);
    attention = assignAttention([pane], attention, []);
    streams.sync(streamPlan(ws, false, { attention }, terminals));
    expect(f.live()).toEqual([]);
  });
  it('follow selection shows the selected agent', () => {
    let ws = workspaceOf(defaultLayout());
    ws = reduce(ws, { type: 'bind', pane: ws.focus!, bind: { follow: 'selection' } });
    expect([...streamPlan(ws, false, { selection: 's', attention: new Map() }, terminals).values()]).toEqual(['s']);
    expect(streamPlan(ws, false, none, terminals).size).toBe(0);
  });
});

describe('follow attention', () => {
  it('several panes show different agents, oldest first', () => {
    expect([...assignAttention(['p1', 'p2', 'p3'], new Map(), ['a', 'b'])]).toEqual([['p1', 'a'], ['p2', 'b']]);
  });
  it('a pane keeps its agent while it still needs you', () => {
    const prev = new Map([['p2', 'b']]);
    expect([...assignAttention(['p1', 'p2'], prev, ['a', 'b'])]).toEqual([['p2', 'b'], ['p1', 'a']]);
  });
  it('moves on when its agent no longer needs you', () => {
    expect([...assignAttention(['p1'], new Map([['p1', 'a']]), ['b', 'c'])]).toEqual([['p1', 'b']]);
  });
});
