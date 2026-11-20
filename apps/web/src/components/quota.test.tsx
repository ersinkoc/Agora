import { beforeEach, describe, expect, it } from 'vitest';
import type { ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { QuotaObs } from '../net/quota';
import { LimitChip, QuotaProvider, QuotaSection, StartLimitHint } from './Quota';
import { UsagePanel } from './UsagePanel';

const agent = (over: Partial<AgentTileModel>): AgentTileModel => ({ id: 'n1/a', name: 'codex · amber', kind: 'codex', accent: '#fff', machine: 'Laptop', machineId: 'n1', workspace: '/w', project: 'w', state: 'idle', preview: '', ...over });
const MIN = 60_000;
const at = Date.now();
const obs = (over: Partial<QuotaObs>): QuotaObs => ({ machineId: 'n1', tool: 'codex', window: 'five_hour', usedPercent: 50, resetsAtMs: at + 80 * MIN, observedAtMs: at - 5 * MIN, ...over });
const names = new Map([['n1', 'Laptop'], ['n2', 'Build server']]);
const draw = (list: QuotaObs[], node: ReactNode) => renderToStaticMarkup(<QuotaProvider obs={list} machines={names}>{node}</QuotaProvider>);

describe('plan limits in the window', () => {
  beforeEach(() => setLanguage('en'));

  it('draws one progress bar per window with values, the reset time and the age', () => {
    const html = draw([obs({ usedPercent: 42 }), obs({ window: 'seven_day', usedPercent: 10 })], <QuotaSection fleet={[agent({})]} />);
    expect(html.match(/role="progressbar"/g)).toHaveLength(2);
    expect(html).toContain('aria-valuenow="42"');
    expect(html).toContain('Five hours');
    expect(html).toContain('Seven days');
    expect(html).toContain('Codex');
    expect(html).toMatch(/Resets in 1 h \d+ min/);
    expect(html).toContain('Observed 5 minutes ago');
    expect(html).toContain('42 %');
    expect(html).not.toMatch(/[$€£]|USD/);
  });

  it('says Reset for a passed window with no old percentage, and Unknown with the reason when nothing was reported', () => {
    const reset = draw([obs({ usedPercent: 88, resetsAtMs: at - MIN })], <QuotaSection fleet={[]} />);
    expect(reset).toContain('Reset');
    expect(reset).not.toContain('88');
    expect(reset).not.toContain('role="progressbar"');
    const unknown = draw([], <QuotaSection fleet={[agent({ kind: 'claude' }), agent({ id: 'n2/a', kind: 'claude-code', machineId: 'n2' }), agent({ id: 'n1/s', kind: 'gemini' })]} />);
    expect(unknown).toContain('No limit information from this tool: Claude, Claude-code');
    expect(unknown.match(/data-state="unknown"/g)).toHaveLength(1);
    expect(unknown).not.toContain('Gemini');
    expect(unknown).not.toContain('role="progressbar"');
    expect(draw([], <QuotaSection fleet={[]} />)).toContain('Plan limits appear here');
  });

  it('marks stale readings and shows amber from 80 and stronger from 95 percent', () => {
    const stale = draw([obs({ window: 'minutes:30', observedAtMs: at - 50 * MIN, resetsAtMs: at + 10 * MIN })], <QuotaSection fleet={[]} />);
    expect(stale).toContain('Out of date');
    expect(stale).toContain('data-state="stale"');
    const high = draw([obs({ usedPercent: 82 })], <QuotaSection fleet={[]} />);
    expect(high).toContain('data-level="high"');
    expect(high).toContain('bg-needs-you');
    expect(draw([obs({ usedPercent: 96 })], <QuotaSection fleet={[]} />)).toContain('data-level="critical"');
    expect(draw([obs({ usedPercent: 40 })], <QuotaSection fleet={[]} />)).toContain('bg-accent');
  });

  it('keeps machines that report the same tool apart', () => {
    const html = draw([obs({ usedPercent: 30 }), obs({ machineId: 'n2', usedPercent: 90 })], <QuotaSection fleet={[]} />);
    expect(html).toContain('Laptop');
    expect(html).toContain('Build server');
    expect(html.match(/role="progressbar"/g)).toHaveLength(2);
  });

  it('sits in the Usage panel, including when nothing else is there to show', () => {
    const html = draw([obs({ usedPercent: 70 })], <UsagePanel fleet={[agent({})]} now={Date.now()} />);
    expect(html).toContain('Plan limits');
    expect(html).toContain('aria-valuenow="70"');
  });

  it('chips an agent only from 80 percent and hints at the start only from 95 without blocking it', () => {
    expect(draw([obs({ usedPercent: 79 })], <LimitChip agent={agent({})} />)).toBe('');
    const chip = draw([obs({ usedPercent: 82 })], <LimitChip agent={agent({})} />);
    expect(chip).toContain('Limit 82 %');
    expect(chip).toContain('role="img"');
    expect(draw([obs({ usedPercent: 94 })], <StartLimitHint tool="codex" machineId="n1" />)).toBe('');
    const hint = draw([obs({ usedPercent: 97 })], <StartLimitHint tool="codex" machineId="n1" />);
    expect(hint).toContain('You can still start.');
    expect(hint).toMatch(/Resets in 1 h/);
    expect(hint).not.toContain('disabled');
    expect(draw([obs({ usedPercent: 97 })], <StartLimitHint tool="claude" machineId="n1" />)).toBe('');
  });

  it('draws a 0 % bar with no fill and a 1 % bar with a visible one, and truncates long names', () => {
    expect(draw([obs({ usedPercent: 0 })], <QuotaSection fleet={[]} />)).toContain('width:0%');
    expect(draw([obs({ usedPercent: 1 })], <QuotaSection fleet={[]} />)).toContain('width:2%');
    expect(draw([obs({ tool: 'a'.repeat(80) })], <QuotaSection fleet={[]} />)).toMatch(/class="block truncate[^"]*" title="A/);
  });

  it('says the same in Turkish', () => {
    setLanguage('tr');
    const html = draw([obs({ usedPercent: 82 })], <QuotaSection fleet={[agent({ kind: 'claude' })]} />);
    expect(html).toContain('Beş saat');
    expect(html).toContain('Limit %82');
    expect(draw([], <QuotaSection fleet={[agent({ kind: 'claude' })]} />)).toContain('Bu araçtan limit bilgisi yok');
    expect(html).toContain('Limit %82');
    setLanguage('en');
  });
});
