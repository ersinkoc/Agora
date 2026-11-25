import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { chipFor, classify, levelOf, machineKeyOf, quotaView, resetsText, startHint, toolName, untilText, windowLabel, windowMinutes, type QuotaObs } from './quota';
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

  it('makes an unknown quota window label visible', () => {
    expect(windowLabel('roll\u3164ing\u202e')).toBe('roll\\u{3164}ing\\u{202e}');
  });

  it('verifies unknown quota window labels across isolate, private-use and control characters', () => {
    expect(windowLabel('burst\u2066')).toBe('burst\\u{2066}');
    expect(windowLabel('custom\ue000\u0001')).toBe('custom\\u{e000}\\u{1}');
  });

  it('makes a quota tool name visible', () => {
    expect(toolName('co\u2800dex\u034f')).toBe('Co\\u{2800}dex\\u{34f}');
  });

  it('verifies quota tool names across zero-width and private-use characters', () => {
    expect(toolName('claude\u200b')).toBe('Claude\\u{200b}');
    expect(toolName('çe\ue000viri')).toBe('Çe\\u{e000}viri');
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
    // No reset time (0) is a reset, never fresh for ever.
    expect(classify(obs({ resetsAtMs: 0 }), NOW).state).toBe('reset');
    expect(quotaView([obs({ usedPercent: 60, resetsAtMs: 0 })], [], new Map(), NOW).tools[0]?.machines[0]?.bars[0]?.percent).toBeUndefined();
    const bar = quotaView([obs({ usedPercent: 99, resetsAtMs: NOW - MIN })], [], new Map(), NOW).tools[0]?.machines[0]?.bars[0];
    expect(bar?.state).toBe('reset');
    expect(bar?.percent).toBeUndefined();
  });

  it('has thresholds at 80 and 95 percent', () => {
    expect([0, 79, 80, 94, 95, 100].map(levelOf)).toEqual(['normal', 'normal', 'high', 'high', 'critical', 'critical']);
  });

  it('lists, as one line, only kinds known to have plan limits that reported nothing; never by display name', () => {
    const fleet = [agent({ name: 'claude · sky', kind: 'claude' }), agent({ id: 'n1/b', name: 'shell · x', kind: 'shell' }), agent({ id: 'n1/c', name: 'x · y', kind: 'gemini' }), agent({ id: 'n1/d', kind: 'claude-code' })];
    const v = quotaView([], fleet, new Map([['n1', 'Laptop']]), NOW);
    expect(v.tools).toEqual([]);
    expect(v.unknown).toEqual(['claude', 'claude-code']);
    // A kind that did report is not listed; an agent with no machine id is skipped (the display name is not an identity).
    expect(quotaView([obs({ tool: 'claude' })], fleet, new Map(), NOW).unknown).toEqual(['claude-code']);
    expect(quotaView([], [agent({ kind: 'codex', machineId: undefined })], new Map(), NOW).unknown).toEqual([]);
    expect(machineKeyOf(agent({ machineId: undefined }))).toBeUndefined();
    expect(machineKeyOf(agent({ machineId: undefined, workspaceRef: { node: 'n9', id: 'w' } }))).toBe('n9');
    expect(chipFor(agent({ kind: 'codex', machineId: undefined, machine: 'n1' }), [obs({ usedPercent: 99 })], NOW)).toBeUndefined();
    expect(quotaView([], [], new Map(), NOW)).toEqual({ tools: [], unknown: [] });
  });

  it("keeps each machine's own plan state apart, windows shortest first", () => {
    const v = quotaView(
      [obs({ machineId: 'n2', usedPercent: 90 }), obs({ window: 'seven_day', usedPercent: 10 }), obs({ usedPercent: 30 }), obs({ window: 'minutes:30', usedPercent: 5 })],
      [], new Map([['n1', 'Laptop'], ['n2', 'Build']]), NOW);
    expect(v.tools).toHaveLength(1);
    expect(v.tools[0]?.machines.map((m) => m.machine)).toEqual(['Build', 'Laptop']);
    expect(v.tools[0]?.machines[1]?.bars.map((b) => [b.window, b.percent])).toEqual([['minutes:30', 5], ['five_hour', 30], ['seven_day', 10]]);
    expect(v.tools[0]?.machines[0]?.bars[0]?.level).toBe('high');
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
    setLanguage('tr');
    expect(untilText(80 * MIN)).toBe('1 sa 20 dk');
    expect(untilText(20 * MIN)).toBe('20 dk');
    expect(untilText(3 * 86_400_000 + 4 * 3_600_000)).toBe('3 gün 4 sa');
    setLanguage('en');
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
    expect(v.quota?.get('n1')?.size).toBe(64);
    // Another machine's windows are not pushed out by a chatty one.
    applyEvent(v, 400, ev('n2', 'n2', 'five_hour', 5, 5));
    expect(quotaObs(v).filter((o) => o.machineId === 'n2')).toHaveLength(1);
    v.nodes.set('n1', { hostname: 'h', os: '', online: true, lastEventMs: 0, agentsRunning: 0, forgotten: true });
    expect(quotaObs(shown(v)).filter((o) => o.machineId === 'n1')).toEqual([]);
  });

  it('cuts a reading dated in the future back to this window\'s clock, so a fast clock cannot win a window for good', () => {
    const v = emptyView();
    const ev = (window: string, pct: number, at: number) => ({ producerId: 'n1', body: 'agentQuotaObserved', agentQuotaObserved: { nodeId: 'n1', tool: 'codex', window, usedPercent: pct, resetsAtMs: Date.now() + 3_600_000, observedAtMs: at } }) as never;
    const day = 86_400_000;
    applyEvent(v, 1, ev('five_hour', 10, Date.now() + day));
    const kept = quotaObs(v)[0];
    expect(kept?.observedAtMs).toBeLessThanOrEqual(Date.now() + 120_000);
    expect(classify({ ...kept!, resetsAtMs: Date.now() + 3_600_000 }, Date.now()).ageMs).toBeLessThanOrEqual(0 + 1);
    // The clock was corrected: an honest reading from just now is newer than the cut one (within the skew), and still applies later.
    applyEvent(v, 2, ev('five_hour', 55, Date.now() + 130_000));
    expect(quotaObs(v)[0]?.usedPercent).toBe(55);
    // A reading older than the cut stored one is dropped, on the cut comparison.
    applyEvent(v, 3, ev('five_hour', 1, Date.now() - 60_000));
    expect(quotaObs(v)[0]?.usedPercent).toBe(55);
  });
});
