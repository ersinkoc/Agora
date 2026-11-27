// ADR-0129: the map's layout is a pure function of the fleet: the hub in the middle, the machines on a ring in name order, the agents
// beyond their machine, a helper further out from its parent; a message travels through the hub between two machines.
import { describe, expect, it } from 'vitest';
import type { AgentTileModel } from '../model';
import { AGENTS_PER_MACHINE, EXTENT, along, fleetMap, fromHub, route, taskSignals } from './fleetMap';

const machine = (id: string, online = true) => ({ id, name: id, os: 'linux', online, agentsRunning: 0, lastSeenMs: 1 });
const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id, name: id, accent: 'var(--agent-teal)', machine: id.split('/')[0] as string, machineId: id.split('/')[0], workspace: 'w', state: 'working', preview: '', ...over,
});

describe('the fleet map', () => {
  it('draws the same fleet the same way, whatever order it comes in', () => {
    const ms = [machine('mac'), machine('blackview'), machine('win', false)];
    const as = [agent('mac/a1'), agent('blackview/b1'), agent('blackview/b2', { parentId: 'blackview/b1' }), agent('win/w1')];
    const a = fleetMap(ms, as);
    const b = fleetMap([...ms].reverse(), [...as].reverse());
    expect(a).toEqual(b);
    expect(a.machines.map((m) => m.id)).toEqual(['blackview', 'mac', 'win']);
    expect(a.machines[0]).toMatchObject({ x: 0, y: -230 }); // the first machine straight above the hub
    const win = a.machines[2]!;
    expect(a.edges.find((e) => e.kind === 'link' && e.to.x === win.x && e.to.y === win.y)?.dim).toBe(true);
  });

  it('puts a helper further out than the agent that started it, and everything inside the frame', () => {
    const m = fleetMap([machine('blackview')], [agent('blackview/b1'), agent('blackview/b2', { parentId: 'blackview/b1' }), agent('blackview/b3', { parentId: 'blackview/b2' })]);
    const r = (id: string) => { const p = m.agents.find((x) => x.id === id)!; return Math.hypot(p.x, p.y); };
    expect(r('blackview/b2')).toBeGreaterThan(r('blackview/b1'));
    expect(r('blackview/b3')).toBeGreaterThan(r('blackview/b2'));
    expect(m.agents.find((x) => x.id === 'blackview/b2')?.parent).toBe('blackview/b1');
    for (const p of m.agents) expect(Math.max(Math.abs(p.x), Math.abs(p.y))).toBeLessThanOrEqual(EXTENT);
  });

  it('counts the agents a crowded machine has beyond what it draws, and leaves closed ones out', () => {
    const many = Array.from({ length: AGENTS_PER_MACHINE + 5 }, (_, i) => agent(`mac/a${String(i).padStart(2, '0')}`));
    const m = fleetMap([machine('mac')], [...many, agent('mac/old', { closed: true })]);
    expect(m.agents).toHaveLength(AGENTS_PER_MACHINE);
    expect(m.machines[0]?.hidden).toBe(5);
    expect(m.agents.some((a) => a.id === 'mac/old')).toBe(false);
  });

  it('routes a message straight on one machine and through the hub between two', () => {
    const m = fleetMap([machine('a'), machine('b')], [agent('a/1'), agent('a/2'), agent('b/1')]);
    expect(route(m, 'a/1', 'a/2')).toHaveLength(2);
    const across = route(m, 'a/1', 'b/1')!;
    expect(across).toHaveLength(5);
    expect(across[2]).toEqual(m.hub);
    expect(route(m, 'a/1', 'nobody')).toBeUndefined();
    expect(along([{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }], 0.75)).toEqual({ x: 10, y: 5 });
    expect(along([{ x: 0, y: 0 }, { x: 10, y: 0 }], 2)).toEqual({ x: 10, y: 0 });
  });

  it('lays out 100 agents quickly', () => {
    const ms = Array.from({ length: 10 }, (_, i) => machine(`m${i}`));
    const as = Array.from({ length: 100 }, (_, i) => agent(`m${i % 10}/a${i}`));
    const at = performance.now();
    for (let i = 0; i < 50; i++) fleetMap(ms, as);
    expect((performance.now() - at) / 50).toBeLessThan(4);
  });

  it('sends a task or a handoff from the hub, once per owner and once per delivery', () => {
    const m = fleetMap([machine('a')], [agent('a/1')]);
    expect(fromHub(m, 'a/1')).toHaveLength(3);
    expect(fromHub(m, '1')?.[2]).toEqual(m.agents[0]); // a machine's own id
    expect(fromHub(m, 'b/9')).toBeUndefined();
    const boards = [{ tasks: [
      { id: 'p/t1', title: 'Fix login', ownerId: 'a/1', handoffs: [{ id: 'h1', receiver: 'a/1', state: 'delivered' }, { id: 'h2', receiver: 'a/1', state: 'waiting' }] },
      { id: 'p/t2', title: 'Unowned' },
    ] }];
    const s = taskSignals(boards);
    expect(s.map((x) => x.key.split('\u0000')[0])).toEqual(['task', 'handoff']);
    expect(s.every((x) => x.to === 'a/1' && x.words === 'Fix login')).toBe(true);
  });
});
