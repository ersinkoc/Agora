// @vitest-environment jsdom
// ADR-0123: every agent says the model it runs on and its reasoning effort, one way for every kind, from what the window was told.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { applyEvent, emptyView, fromSnapshot, modelAndEffort, providerOf, tiles } from '../net/view';
import { matchesModelPick, modelPicks } from '../net/fleet';
import { readKnown } from '../net/agentKinds';
import { demoAgents, demoMachines } from '../fixtures';
import { FleetConsole } from './FleetConsole';
import { AgentTile } from './AgentTile';
import { ModelChip, SharedChip, providerName } from './ModelChip';

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

  it("name the provider from the kind's manifest, else from a provider/model id", () => {
    expect(providerOf('anthropic', 'opus')).toBe('anthropic');
    expect(providerOf('', 'openrouter/qwen-3')).toBe('openrouter');
    expect(providerOf(undefined, 'opus')).toBe('');
    expect(providerOf('Not Valid!', 'Bad Name/x')).toBe('');
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'mac', body: 'agentStarted', agentStarted: { agentId: 'agent-1', nodeId: 'mac', kind: 'claude-code', launchSettings: [{ id: 'model', value: 'opus' }] } } as never);
    const [a] = tiles(v, undefined, (node, kind) => (node === 'mac' && kind === 'claude-code' ? 'anthropic' : undefined));
    expect(a?.provider).toBe('anthropic');
    expect(providerName('anthropic')).toBe('Anthropic');
    expect(providerName('acme-ai')).toBe('acme-ai');
  });

  it("keep a kind's provider in this browser's list, and drop one the rules refuse", () => {
    const store = { getItem: () => JSON.stringify({ mac: { atMs: 1, kinds: [
      { kind: 'claude-code', name: 'Claude Code', provider: 'anthropic' }, { kind: 'odd', name: 'Odd', provider: 'Evil Corp\u202e' }] } }) };
    const kinds = readKnown(store).get('mac')?.kinds ?? [];
    expect(kinds.map((k) => k.provider)).toEqual(['anthropic', '']);
  });

  it("say the provider in the chip's words, and take no focus inside a tile", () => {
    act(() => root.render(<ModelChip agent={{ model: 'opus', effort: 'high', provider: 'anthropic' }} nested />));
    const chip = host.querySelector('[data-model-chip]')!;
    expect(chip.getAttribute('aria-label')).toBe('Provider: Anthropic · Model: opus · Reasoning effort: high');
    expect(chip.getAttribute('tabindex')).toBeNull();
    act(() => root.render(<AgentTile agent={demoAgents[0]!} context={{ group: 'machine' }} />));
    expect(host.querySelector('.tile [data-model-chip]')?.textContent).toBe('opus · high');
  });

  it('narrow the fleet to one provider or one model', () => {
    const agents = [{ provider: 'anthropic', model: 'opus' }, { provider: 'anthropic', model: 'sonnet' }, { provider: 'openai', model: 'gpt-6' }, {}];
    const picks = modelPicks(agents);
    expect(picks.providers.map((p) => [p.value, p.count])).toEqual([['p:anthropic', 2], ['p:openai', 1]]);
    expect(picks.models.map((m) => m.value)).toEqual(['m:gpt-6', 'm:opus', 'm:sonnet']);
    expect(agents.filter((a) => matchesModelPick(a, 'p:anthropic'))).toHaveLength(2);
    expect(agents.filter((a) => matchesModelPick(a, 'm:gpt-6'))).toHaveLength(1);
    expect(agents.filter((a) => matchesModelPick(a, undefined))).toHaveLength(4);
    act(() => root.render(<FleetConsole fleet={demoAgents} approvals={[]} machines={demoMachines} link="live" now={Date.now()} onOpen={() => {}} />));
    const select = host.querySelector('.fleet-tools__pick select') as HTMLSelectElement;
    expect(select).toBeTruthy();
    const tilesShown = () => host.querySelectorAll('.fleet-grid .tile').length;
    const all = tilesShown();
    act(() => { select.value = 'p:openai'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(tilesShown()).toBe(1);
    act(() => { select.value = ''; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(tilesShown()).toBe(all);
  });
});

// ADR-0122 decision 7: an agent's pane says which shared servers it was started with, from its start and from the hub's snapshot.
describe('shared with agents', () => {
  it('comes from the start and from the snapshot, and a new start replaces it', () => {
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'mac', body: 'agentStarted', agentStarted: { agentId: 'agent-1', nodeId: 'mac', kind: 'claude-code', sharedItems: [{ name: 'db', itemHash: 'h1' }, { name: '', itemHash: 'x' }, { name: 'docs', itemHash: 'h2' }] } } as never);
    expect(tiles(v).find((x) => x.id === 'mac/agent-1')?.sharedItems).toEqual(['db', 'docs']);
    applyEvent(v, 2, { producerId: 'mac', body: 'agentStarted', agentStarted: { agentId: 'agent-1', nodeId: 'mac', kind: 'claude-code' } } as never);
    expect(tiles(v).find((x) => x.id === 'mac/agent-1')?.sharedItems).toBeUndefined();
    const s = fromSnapshot({ agents: [{ agentId: 'mac/agent-3', nodeId: 'mac', kind: 'claude-code', running: true, sharedItems: [{ name: 'db', itemHash: 'h1' }] }] } as never);
    expect(tiles(s).find((x) => x.id === 'mac/agent-3')?.sharedItems).toEqual(['db']);
  });

  it('shows a count with the names in words, and nothing without any', () => {
    act(() => root.render(<SharedChip agent={{ sharedItems: ['db', 'docs\u202e'] }} />));
    const chip = host.querySelector('[data-shared-chip]');
    expect(chip?.textContent).toBe('2 shared');
    expect(chip?.getAttribute('aria-label')).toContain('db, docs');
    expect(chip?.getAttribute('aria-label')).not.toContain('\u202e');
    act(() => root.render(<SharedChip agent={{}} />));
    expect(host.querySelector('[data-shared-chip]')).toBeNull();
  });
});
