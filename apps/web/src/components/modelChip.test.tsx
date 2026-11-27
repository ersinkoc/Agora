// @vitest-environment jsdom
// ADR-0123: every agent says the model it runs on and its reasoning effort, one way for every kind, from what the window was told.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { applyEvent, emptyView, fromSnapshot, modelAndEffort, tiles } from '../net/view';
import { ModelChip } from './ModelChip';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('model and effort', () => {
  it('come from the adapter first, then the launch settings, then the usage', () => {
    const settings = [{ id: 'model', value: 'opus' }, { id: 'effort', value: 'high' }];
    expect(modelAndEffort({ launchSettings: settings })).toEqual({ model: 'opus', effort: 'high' });
    expect(modelAndEffort({ usageModel: 'gpt-6' })).toEqual({ model: 'gpt-6', effort: '' });
    const session = {
      modes: [], models: [{ id: 'large', name: 'Opus 5.5', description: '', skipsPermissions: false }], currentMode: '', currentModel: 'large',
      contextTokens: 0, contextWindow: 0,
      config: [{ id: 't', name: 'Thinking', description: '', category: 'thought_level' as const, type: 'select' as const, current: 'max',
        choices: [{ id: 'max', name: 'Max', description: '', skipsPermissions: false }], skipsPermissions: false }],
    };
    expect(modelAndEffort({ session, launchSettings: settings, usageModel: 'x' })).toEqual({ model: 'Opus 5.5', effort: 'Max' });
    expect(modelAndEffort({})).toEqual({ model: '', effort: '' });
  });

  it('reach the tile from a start, from usage and from a snapshot', () => {
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'mac', body: 'agentStarted', agentStarted: { agentId: 'agent-1', nodeId: 'mac', kind: 'claude-code', launchSettings: [{ id: 'model', value: 'opus' }, { id: 'effort', value: 'high' }] } } as never);
    applyEvent(v, 2, { producerId: 'mac', body: 'agentStarted', agentStarted: { agentId: 'agent-2', nodeId: 'mac', kind: 'codex' } } as never);
    applyEvent(v, 3, { producerId: 'mac', body: 'agentUsageReported', agentUsageReported: { agentId: 'agent-2', model: 'gpt-6' } } as never);
    const shown = (id: string) => { const a = tiles(v).find((x) => x.id === id)!; return [a.model, a.effort]; };
    expect(shown('mac/agent-1')).toEqual(['opus', 'high']);
    expect(shown('mac/agent-2')).toEqual(['gpt-6', '']);
    const s = fromSnapshot({ agents: [{ agentId: 'mac/agent-3', nodeId: 'mac', kind: 'codex', running: true, model: 'gpt-6', launchSettings: [{ id: 'effort', value: 'low' }] }] } as never);
    const a = tiles(s).find((x) => x.id === 'mac/agent-3')!;
    expect([a.model, a.effort]).toEqual(['gpt-6', 'low']);
  });

  it('show as one short chip with the whole of both as its name, and nothing when neither is known', () => {
    act(() => root.render(<ModelChip agent={{ model: 'opus', effort: 'high' }} />));
    const chip = host.querySelector('[data-model-chip]')!;
    expect(chip.textContent).toBe('opus · high');
    expect(chip.getAttribute('aria-label')).toBe('Model: opus · Reasoning effort: high');
    act(() => root.render(<ModelChip agent={{ effort: 'low' }} />));
    expect(host.querySelector('[data-model-chip]')!.textContent).toBe('effort low');
    act(() => root.render(<ModelChip agent={{ model: 'a\u001b[2Jb' }} />));
    expect(host.querySelector('[data-model-chip]')!.textContent).not.toContain('\u001b');
    act(() => root.render(<ModelChip agent={{}} />));
    expect(host.querySelector('[data-model-chip]')).toBeNull();
  });
});
