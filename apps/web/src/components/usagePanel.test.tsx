import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { activeMs, hasUsage, usageView } from '../net/usageView';
import { loadRailTab, railItems } from './rail';
import { UsagePanel } from './UsagePanel';

const agent = (over: Partial<AgentTileModel>): AgentTileModel => ({ id: 'n1/a', name: 'claude · amber', accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: '/w/app', project: 'app', projectId: 'p1', state: 'idle', preview: '', tokens: 0, ...over });
const fleet = [
  agent({ id: 'n1/a', tokens: 12_400, startedMs: 1_000, session: { modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 50, contextWindow: 100, costMicros: 20_000, currency: 'USD' } }),
  agent({ id: 'n2/b', name: 'codex · plum', machine: 'vps', machineId: 'n2', tokens: 800, startedMs: 2_000, closed: true, endedMs: 62_000 }),
];

describe('usage (no money)', () => {
  beforeEach(() => setLanguage('en'));
  it('adds tokens and time per machine and project, heaviest first', () => {
    const v = usageView(fleet, 61_000);
    expect(v.totalTokens).toBe(13_200);
    expect(v.byMachine.map((m) => m.machine)).toEqual(['laptop', 'vps']);
    expect(v.byProject[0]?.agents[0]?.contextPct).toBe(50);
    expect(activeMs(fleet[1] as AgentTileModel, 99_999_999)).toBe(60_000); // a closed agent stops at its end
    expect(activeMs(agent({}), 5)).toBeUndefined(); // a start this window did not see is unknown, not zero
  });
  it('shows the rail subject only when there is something to measure, and still reads the old saved name', () => {
    expect(hasUsage([agent({})])).toBe(false);
    expect(hasUsage(fleet)).toBe(true);
    expect(railItems({ needs: 0, changed: 0, messages: 0, hasBoard: false, hasUsage: true }).some((x) => x.id === 'usage')).toBe(true);
    expect(loadRailTab({ getItem: () => 'cost' })).toBe('usage');
  });
  it('never draws a currency amount, even when an agent reported a cost', () => {
    const html = renderToStaticMarkup(<UsagePanel fleet={fleet} now={61_000} />);
    expect(html).not.toMatch(/\$|USD|<details|reported by the agent/i);
    expect(html).toContain('Plan limits appear here when the agent reports them.');
    expect(html).toContain('13.2K tokens used');
  });
  it('says one token, not "1 tokens", and keys machines by id so two with one name stay apart', () => {
    expect(renderToStaticMarkup(<UsagePanel fleet={[agent({ tokens: 1, startedMs: 1 })]} now={5} />)).toContain('1 token used');
    const v = usageView([agent({ id: 'n1/a', machineId: 'n1', tokens: 5 }), agent({ id: 'n2/b', machineId: 'n2', tokens: 7 })], 10);
    expect(v.byMachine.map((m) => m.id)).toEqual(['n2', 'n1']);
    expect(v.byMachine).toHaveLength(2);
  });
  it('stops counting time at the end of a closed agent', () => {
    expect(activeMs(agent({ startedMs: 1_000, closed: true, endedMs: 5_000 }), 999_999)).toBe(4_000);
    expect(activeMs(agent({ startedMs: 1_000 }), 6_000)).toBe(5_000);
  });
  it('says plainly when there is nothing yet, and keeps the plan limits place', () => {
    const html = renderToStaticMarkup(<UsagePanel fleet={[agent({})]} now={1} />);
    expect(html).toContain('Nothing to show yet');
    expect(html).toContain('Plan limits');
    expect(html).not.toContain('<details');
  });
});
