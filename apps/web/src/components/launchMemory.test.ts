import { describe, expect, it } from 'vitest';
import { recall, remember, rememberStarted, startedWith } from './launchMemory';
import type { LaunchSettingChoice } from './launchSettingsModel';

const effort: LaunchSettingChoice = { id: 'effort', label: 'setting-effort', values: ['low', 'high'], defaultValue: 'low' };
const model: LaunchSettingChoice = { id: 'model', label: 'setting-model', values: [], defaultValue: '' };
const memory = () => { const m = new Map<string, string>(); return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) }; };

describe('remembered start settings (ADR-0066 decision 3)', () => {
  it('gives back the last choice per kind per machine', () => {
    const s = memory();
    remember('n1', 'codex', [['model', 'gpt-x'], ['effort', 'high']], [model, effort], s);
    remember('n2', 'codex', [['effort', 'low']], [model, effort], s);
    expect(recall('n1', 'codex', [model, effort], s)).toEqual({ model: 'gpt-x', effort: 'high' });
    expect(recall('n2', 'codex', [model, effort], s)).toEqual({ effort: 'low' });
    expect(recall('n1', 'claude', [model, effort], s)).toEqual({});
  });
  it('drops a value the machine no longer lists, and a setting it no longer has', () => {
    const s = memory();
    remember('n1', 'codex', [['model', 'gpt-x'], ['effort', 'high']], [model, effort], s);
    expect(recall('n1', 'codex', [model, { ...effort, values: ['low', 'medium'] }], s)).toEqual({ model: 'gpt-x' });
    expect(recall('n1', 'codex', [effort], s)).toEqual({ effort: 'high' });
  });
  it('never remembers a value that was not allowed, and starting with defaults forgets', () => {
    const s = memory();
    remember('n1', 'codex', [['effort', 'insane'], ['mystery', 'x']], [effort], s);
    expect(recall('n1', 'codex', [effort], s)).toEqual({});
    remember('n1', 'codex', [['effort', 'high']], [effort], s);
    remember('n1', 'codex', [], [effort], s);
    expect(recall('n1', 'codex', [effort], s)).toEqual({});
  });
  it('survives blocked or corrupt storage', () => {
    expect(recall('n1', 'codex', [effort], undefined)).toEqual({});
    const bad = { getItem: () => '{not json', setItem: () => { throw new Error('full'); } };
    expect(recall('n1', 'codex', [effort], bad)).toEqual({});
    expect(() => remember('n1', 'codex', [['effort', 'high']], [effort], bad)).not.toThrow();
    const odd = { getItem: () => '{"n1":{"codex":{"effort":5}}}', setItem: () => undefined };
    expect(recall('n1', 'codex', [effort], odd)).toEqual({});
  });
  it('keeps what an agent was started with, per agent', () => {
    const s = memory();
    rememberStarted('n1/a1', [['effort', 'high']], s);
    expect(startedWith('n1/a1', s)).toEqual([['effort', 'high']]);
    expect(startedWith('n1/a2', s)).toEqual([]);
    rememberStarted('n1/a3', [], s);
    expect(startedWith('n1/a3', s)).toEqual([]);
  });
});
