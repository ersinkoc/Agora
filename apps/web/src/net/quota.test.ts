import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { chipFor, classify, levelOf, quotaView, resetsText, startHint, untilText, windowLabel, windowMinutes, type QuotaObs } from './quota';
import { applyEvent, emptyView, quotaObs, shown } from './view';

const MIN = 60_000;
const NOW = 1_700_000_000_000;
const agent = (over: Partial<AgentTileModel>): AgentTileModel => ({ id: 'n1/a', name: 'codex · amber', accent: '#fff', machine: 'Laptop', machineId: 'n1', workspace: '/w', project: 'w', state: 'idle', preview: '', ...over });
const obs = (over: Partial<QuotaObs>): QuotaObs => ({ machineId: 'n1', tool: 'codex', window: 'five_hour', usedPercent: 50, resetsAtMs: NOW + 60 * MIN, observedAtMs: NOW - 5 * MIN, ...over });

describe('plan limits (ADR-0091)', () => {
  beforeEach(() => setLanguage('en'));

  it('derives the window and its label from the length, never from a field name', () => {
    expect(windowMinutes('five_hour')).toBe(300);
    expect(windowMinutes('seven_day')).toBe(10080);
    expect(windowMinutes('minutes:45')).toBe(45);
    expect(windowMinutes('primary')).toBeUndefined();
    expect(windowMinutes('minutes:0')).toBeUndefined();
    expect(windowLabel('five_hour')).toBe('Five hours');
    expect(windowLabel('seven_day')).toBe('Seven days');
    expect(windowLabel('minutes:45')).toBe('45 minutes');
    expect(windowLabel('minutes:1')).toBe('1 minute');
  });

  it('classifies fresh, stale and reset, and a reset never keeps its old percentage', () => {
    expect(classify(obs({}), NOW).state).toBe('fresh');
    // Older than the window itself: stale.
    expect(classify(obs({ window: 'minutes:30', observedAtMs: NOW - 31 * MIN, resetsAtMs: NOW + MIN }), NOW).state).toBe('stale');
    expect(classify(obs({ window: 'minutes:30', observedAtMs: NOW - 29 * MIN, resetsAtMs: NOW + MIN }), NOW).state).toBe('fresh');
    // The reset time passed: reset, however fresh the reading.
    expect(classify(obs({ resetsAtMs: NOW - 1 }), NOW).state).toBe('reset');
    expect(classify(obs({ resetsAtMs: NOW }), NOW).state).toBe('reset');
    // A reading from the future (a machine's clock is ahead) is just now, never a negative age.
    expect(classify(obs({ observedAtMs: NOW + 10 * MIN }), NOW).ageMs).toBe(0);
    const [tool] = quotaView([obs({ usedPercent: 99, resetsAtMs: NOW - MIN })], [], new Map(), NOW);
    const bar = tool?.machines[0]?.bars[0];
    expect(bar?.state).toBe('reset');
    expect(bar?.percent).toBeUndefined();
  });

  it('has thresholds at 80 and 95 percent', () => {
    expect([0, 79, 80, 94, 95, 100].map(levelOf)).toEqual(['normal', 'normal', 'high', 'high', 'critical', 'critical']);
  });

  it('shows a tool with agents and no observation as unknown, and invents nothing', () => {
    const v = quotaView([], [agent({ name: 'claude · sky', kind: 'claude' }), agent({ id: 'n1/b', name: 'shell · x', kind: 'shell' })], new Map([['n1', 'Laptop']]), NOW);
    expect(v).toHaveLength(1);
    expect(v[0]?.tool).toBe('claude');
    expect(v[0]?.machines[0]?.bars).toMatchObject([{ state: 'unknown', machine: 'Laptop' }]);
    expect(v[0]?.machines[0]?.bars[0]?.percent).toBeUndefined();
    expect(quotaView([], [], new Map(), NOW)).toEqual([]);
  });

  it("keeps each machine's own plan state apart, windows shortest first", () => {
    const v = quotaView(
      [obs({ machineId: 'n2', usedPercent: 90 }), obs({ window: 'seven_day', usedPercent: 10 }), obs({ usedPercent: 30 }), obs({ window: 'minutes:30', usedPercent: 5 })],
      [], new Map([['n1', 'Laptop'], ['n2', 'Build']]), NOW);
    expect(v).toHaveLength(1);
    expect(v[0]?.machines.map((m) => m.machine)).toEqual(['Build', 'Laptop']);
    expect(v[0]?.machines[1]?.bars.map((b) => [b.window, b.percent])).toEqual([['minutes:30', 5], ['five_hour', 30], ['seven_day', 10]]);
    expect(v[0]?.machines[0]?.bars[0]?.level).toBe('high');
  });

  it("chips only a fresh window from 80 percent of the agent's own tool and machine; the start hint only from 95", () => {
    const list = [obs({ usedPercent: 82 }), obs({ window: 'seven_day', usedPercent: 96, resetsAtMs: NOW - 1 }), obs({ machineId: 'n2', usedPercent: 99 }), obs({ tool: 'claude', usedPercent: 99 })];
    expect(chipFor(agent({ kind: 'codex' }), list, NOW)).toMatchObject({ percent: 82, level: 'high', window: 'five_hour' });
    expect(chipFor(agent({ kind: 'codex' }), [obs({ usedPercent: 79 })], NOW)).toBeUndefined();
    expect(chipFor(agent({ kind: 'codex' }), [obs({ usedPercent: 90, observedAtMs: NOW - 400 * MIN })], NOW)).toBeUndefined(); // stale
    expect(startHint('codex', 'n1', list, NOW)).toBeUndefined();
    expect(startHint('codex', 'n2', list, NOW)).toMatchObject({ percent: 99 });
    expect(startHint('codex', 'n1', [obs({ usedPercent: 95 })], NOW)?.percent).toBe(95);
  });

  it('writes the time to a reset without seconds, and never a currency', () => {
    expect(untilText(80 * MIN)).toBe('1 h 20 min');
    expect(untilText(20 * MIN)).toBe('20 min');
    expect(untilText(10)).toBe('1 min');
    expect(untilText(3 * 86_400_000 + 4 * 3_600_000)).toBe('3 d 4 h');
    expect(resetsText(NOW + 80 * MIN, NOW)).toMatch(/^Resets in 1 h 20 min \(/);
    expect(resetsText(NOW + 80 * MIN, NOW)).not.toMatch(/[$€£]/);
  });

  it('folds observations into the view: the machine speaks for itself, the newest wins, the table is bounded, a forgotten machine is hidden', () => {
    const v = emptyView();
    const ev = (producer: string, node: string, window: string, pct: number, at: number) => ({ producerId: producer, body: 'agentQuotaObserved', agentQuotaObserved: { nodeId: node, tool: 'codex', window, usedPercent: pct, resetsAtMs: NOW + MIN, observedAtMs: at } }) as never;
    applyEvent(v, 1, ev('n1', 'n1', 'five_hour', 10, 100));
    applyEvent(v, 2, ev('n1', 'n1', 'five_hour', 20, 200));
    applyEvent(v, 3, ev('n1', 'n1', 'five_hour', 5, 150)); // older: ignored
    applyEvent(v, 4, ev('n2', 'n1', 'five_hour', 99, 300)); // names another machine: ignored
    applyEvent(v, 5, ev('n1', 'n1', 'seven_day', 300, 100)); // clamped
    expect(quotaObs(v).map((o) => [o.window, o.usedPercent])).toEqual([['five_hour', 20], ['seven_day', 100]]);
    // The replay after a snapshot comes at the snapshot's own position and still applies.
    applyEvent(v, 5, ev('n1', 'n1', 'five_hour', 30, 400));
    expect(quotaObs(v)[0]?.usedPercent).toBe(30);
    for (let i = 0; i < 300; i++) applyEvent(v, 6 + i, ev('n1', 'n1', `minutes:${i + 1}`, 1, 1000 + i));
    expect(v.quota?.size).toBe(256);
    v.nodes.set('n1', { hostname: 'h', os: '', online: true, lastEventMs: 0, agentsRunning: 0, forgotten: true });
    expect(quotaObs(shown(v))).toEqual([]);
  });
});
