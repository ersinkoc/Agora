// @vitest-environment jsdom
// ADR-0129: the map draws the machines and their agents, opens an agent on a click or Enter, moves only the messages that arrive while
// it is open, and moves nothing under reduced motion.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { MessageModel } from '../net/view';
import { FleetMap } from './FleetMap';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const machines = [{ id: 'mac', name: 'mac', os: 'macos', online: true, agentsRunning: 1, lastSeenMs: 1 }, { id: 'win', name: 'win', os: 'windows', online: false, agentsRunning: 0, lastSeenMs: 1 }];
const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id, name: id.replace('/', ' · '), accent: 'var(--agent-teal)', machine: id.split('/')[0] as string, machineId: id.split('/')[0], workspace: 'w', state: 'working', preview: '', ...over });
const fleet = [agent('mac/a1', { state: 'needs-you' }), agent('win/w1')];
const msg = (body: string, atMs: number): MessageModel => ({ from: 'mac · a1', to: 'win · w1', body, atMs, crossMachine: true, fromId: 'mac/a1', toId: 'win/w1' });

let root: Root;
let host: HTMLDivElement;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

describe('the fleet map', () => {
  it('draws every machine and agent, and opens an agent on a click or Enter', () => {
    const onOpen = vi.fn();
    act(() => root.render(<FleetMap machines={machines} fleet={fleet} messages={[]} onOpen={onOpen} />));
    expect(host.querySelector('svg')?.getAttribute('aria-label')).toBe('A map of 2 machines and 2 agents');
    expect(host.querySelectorAll('.fleet-map__machine')).toHaveLength(2);
    expect(host.querySelector('.fleet-map__machine--off title')?.textContent).toBe('win (offline)');
    expect(host.querySelector('a.fleet-map__machine')?.getAttribute('href')).toBe('#/machines/mac');
    const a1 = host.querySelector('[aria-label="mac · a1, Needs you"]') as SVGGElement;
    expect(a1.getAttribute('class')).toContain('fleet-map__agent--needs-you');
    act(() => { a1.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    act(() => { a1.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); });
    expect(onOpen).toHaveBeenCalledTimes(2);
    expect(onOpen).toHaveBeenCalledWith('mac/a1');
  });

  it('moves a message that arrives while it is open, not the ones it found', () => {
    vi.stubGlobal('requestAnimationFrame', () => 1);
    vi.stubGlobal('cancelAnimationFrame', () => {});
    act(() => root.render(<FleetMap machines={machines} fleet={fleet} messages={[msg('old', 1)]} onOpen={() => {}} />));
    expect(host.querySelectorAll('.fleet-map__dot')).toHaveLength(0);
    act(() => root.render(<FleetMap machines={machines} fleet={fleet} messages={[msg('new', 2), msg('old', 1)]} onOpen={() => {}} />));
    const dots = host.querySelectorAll('.fleet-map__dot');
    expect(dots).toHaveLength(1);
    expect(dots[0]?.querySelector('title')?.textContent).toBe('mac · a1 to win · w1: new');
  });

  it('moves a task given to an agent while it is open, as a card from the hub', () => {
    vi.stubGlobal('requestAnimationFrame', () => 1);
    vi.stubGlobal('cancelAnimationFrame', () => {});
    const board = (owner?: string) => [{ projectId: 'p', project: 'p', folders: [], leases: [], tasks: [{ id: 'p/t1', title: 'Fix login', status: 'claimed' as const, owner: owner ?? '', ...(owner ? { ownerId: owner } : {}) }] }];
    act(() => root.render(<FleetMap machines={machines} fleet={fleet} messages={[]} boards={board()} onOpen={() => {}} />));
    expect(host.querySelectorAll('.fleet-map__card')).toHaveLength(0);
    act(() => root.render(<FleetMap machines={machines} fleet={fleet} messages={[]} boards={board('mac/a1')} onOpen={() => {}} />));
    const cards = host.querySelectorAll('.fleet-map__card');
    expect(cards).toHaveLength(1);
    expect(cards[0]?.querySelector('title')?.textContent).toBe('Task: Fix login');
  });

  it('flashes the recipient instead of moving anything under reduced motion', () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('reduce'), addEventListener() {}, removeEventListener() {} }));
    act(() => root.render(<FleetMap machines={machines} fleet={fleet} messages={[]} onOpen={() => {}} />));
    act(() => root.render(<FleetMap machines={machines} fleet={fleet} messages={[msg('hi', 3)]} onOpen={() => {}} />));
    expect(host.querySelectorAll('.fleet-map__dot')).toHaveLength(0);
    expect(host.querySelector('.fleet-map__agent--flash')?.getAttribute('aria-label')).toContain('win · w1');
  });

  it('says so when there is no machine yet', () => {
    act(() => root.render(<FleetMap machines={[]} fleet={[]} messages={[]} onOpen={() => {}} />));
    expect(host.textContent).toContain('No machines connected yet.');
  });
});
