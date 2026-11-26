import { describe, expect, it } from 'vitest';
import type { AgentTileModel } from '../model';
import { agentKindOf, groupFleet, projectKeyOf, projectSpread, sharedAcross, sortByUrgency, stateGroupOf } from './fleet';
import { parseGroupBy } from './fleetPrefs';

const agent = (id: string, o: Partial<AgentTileModel> = {}): AgentTileModel => ({ id, name: `claude · ${id}`, accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: 'app', state: 'idle', preview: '', ...o });

describe('group by agent kind and state', () => {
  const fleet = sortByUrgency([
    agent('a', { name: 'claude · amber', state: 'working' }), agent('b', { name: 'codex · teal', state: 'needs-you' }), agent('c', { name: 'claude · rose', state: 'finished' }),
    agent('d', { name: 'wrongstack · sand', kind: 'wrongstack', state: 'working' }), agent('e', { name: 'x', state: 'idle' }),
    agent('f', { name: 'claude · olive', state: 'finished', closed: true, exitCode: 1 }),
  ]);
  it('reads the kind from the node-reported kind, else from the generated name, and has none for a bare name', () => {
    expect(agentKindOf(agent('a', { name: 'claude · amber' }))).toBe('claude');
    expect(agentKindOf(agent('a', { name: 'whatever', kind: 'codex' }))).toBe('codex');
    expect(agentKindOf(agent('a', { name: 'x' }))).toBe('');
  });
  it('groups by kind, the nameless last', () => {
    const g = groupFleet(fleet, [], 'kind');
    expect(g.map((x) => x.name)).toEqual(['codex', 'claude', 'wrongstack', '']); // the group with something that needs you comes first
    expect(g.find((x) => x.name === 'claude')?.agents.map((a) => a.id).sort()).toEqual(['a', 'c', 'f']);
  });
  it('makes a node-reported kind visible in the group name while retaining its raw key', () => {
    const raw = 'co\u3164dex\u202e';
    const [group] = groupFleet([agent('unsafe', { kind: raw })], [], 'kind');
    expect(group).toMatchObject({ key: raw, name: 'co\\u{3164}dex\\u{202e}' });
  });
  it('verifies kind group names across isolate, private-use and control characters', () => {
    const kinds = ['cla\u2066ude', 'çe\ue000viri\u0001'];
    const groups = groupFleet(kinds.map((kind, i) => agent(String(i), { kind })), [], 'kind');
    expect(new Map(groups.map((g) => [g.key, g.name]))).toEqual(new Map([
      [kinds[0], 'cla\\u{2066}ude'], [kinds[1], 'çe\\u{e000}viri\\u{1}'],
    ]));
  });
  it('groups by state in a fixed order, what needs you first', () => {
    const g = groupFleet(fleet, [], 'state');
    expect(g.map((x) => x.key)).toEqual(['needs-you', 'working', 'idle', 'finished', 'failed']);
    expect(g.map((x) => x.labelKey)).toEqual(['state-needs-you', 'state-working', 'state-idle', 'state-finished', 'state-failed']);
    expect(stateGroupOf(agent('z', { state: 'unknown' }))).toBe('idle');
  });
});

describe('the same project on several machines', () => {
  const fleet = sortByUrgency([
    agent('a', { projectId: 'P1', project: 'app', machineId: 'n1', machine: 'laptop', filesChanged: 2 }),
    agent('b', { projectId: 'P1', project: 'app', machineId: 'n2', machine: 'server', filesChanged: 1 }),
    agent('c', { projectId: 'P1', project: 'app', machineId: 'n2', machine: 'server' }),
    agent('d', { projectId: 'P2', project: 'app', machineId: 'n3', machine: 'pc' }), // same folder name, another project
    agent('e', { project: 'app', machineId: 'n4', machine: 'vps' }), // no identity
    agent('f', { project: 'app', machineId: 'n5', machine: 'mac' }), // no identity, same folder name, another machine
  ]);
  it('is one group by the hub project id, and a shared folder name alone never merges anything', () => {
    const g = groupFleet(fleet, [], 'project');
    expect(g).toHaveLength(4);
    expect(g.find((x) => x.key === 'P1')?.agents.map((a) => a.id).sort()).toEqual(['a', 'b', 'c']);
    expect(g.find((x) => x.key === 'P2')?.agents.map((a) => a.id)).toEqual(['d']);
    expect(projectKeyOf(fleet.find((a) => a.id === 'e') as AgentTileModel)).not.toBe(projectKeyOf(fleet.find((a) => a.id === 'f') as AgentTileModel));
  });
  it('counts the machines and agents of a project, and the machines with changes', () => {
    const s = projectSpread(fleet);
    expect(s.get('P1')).toEqual({ machines: ['laptop', 'server'], agents: 3, changing: 2 });
    expect(sharedAcross(s.get('P1'))).toBe(true);
    expect(sharedAcross(s.get('P2'))).toBe(false); // one machine: not "the same project on several"
    expect(s.size).toBe(2); // agents without an identity are never spread
  });
  it('ignores closed agents', () => {
    const s = projectSpread([agent('a', { projectId: 'P', machineId: 'n1' }), agent('b', { projectId: 'P', machineId: 'n2', closed: true })]);
    expect(sharedAcross(s.get('P'))).toBe(false);
  });
});

describe('agents without a project id', () => {
  it('are grouped by the folder they run in, never into several groups with one title', () => {
    const fleet = sortByUrgency([agent('a', { workspace: '/srv/api' }), agent('b', { workspace: '/srv/web/' }), agent('c', { workspace: '/srv/api' }), agent('d', { workspace: 'D:\\code\\site', machineId: 'n2', machine: 'pc' })]);
    const g = groupFleet(fleet, [], 'project');
    expect(g.map((x) => x.name).sort()).toEqual(['api', 'site', 'web']);
    expect(new Set(g.map((x) => x.name)).size).toBe(g.length);
    expect(g.find((x) => x.name === 'api')?.agents).toHaveLength(2);
  });
});

describe('the remembered grouping', () => {
  it('knows the four choices and falls back to machine', () => {
    for (const g of ['machine', 'project', 'kind', 'state']) expect(parseGroupBy(g)).toBe(g);
    expect(parseGroupBy('rows')).toBe('machine');
  });
});
