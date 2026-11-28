// @vitest-environment jsdom
// Naming an agent from the agent list (net/agentNames.ts): the pencil or F2 turns the row's name into a field; Enter or leaving it keeps
// the name, Escape keeps the old one, an empty field gives back the automatic name. The name is this browser's, kept in localStorage.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { fromSnapshot, tiles, type MachineModel } from '../net/view';
import { agora } from '../gen/agora.js';
import { agentNameOf, cleanAgentName, MAX_AGENT_NAMES, parseAgentNames, resetAgentNames, setAgentName, useAgentNamesVersion, withOwnNames } from '../net/agentNames';
import { Explorer } from './Explorer';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); localStorage.clear(); resetAgentNames(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); localStorage.clear(); resetAgentNames(); });

const base = { machine: 'Laptop', machineId: 'laptop', workspace: 'proj', project: 'proj', preview: '' } as const;
const fleet: AgentTileModel[] = [
  { ...base, id: 'laptop/a1', name: 'claude · amber', kind: 'claude', accent: 'var(--agent-amber)', state: 'working' },
  { ...base, id: 'laptop/a2', name: 'codex · teal', accent: 'var(--agent-teal)', state: 'idle' },
];
const machines = [{ id: 'laptop', name: 'Laptop', os: 'linux', online: true, agentsRunning: 2, lastSeenMs: Date.now() }] as MachineModel[];
const noop = () => {};

// What App does: the list is rebuilt from the names whenever one changes.
function List() {
  const v = useAgentNamesVersion();
  return <Explorer by="machine" onBy={noop} spread={new Map()} fleet={withOwnNames(fleet, v)} machines={machines} open={[]} projects={[]} project=""
    onProject={noop} onToggle={noop} onCollapse={noop} onDismiss={noop} />;
}
const render = () => act(() => root.render(<List />));
const nameOf = (id: string) => host.querySelector(`[data-agent-row="${id}"] [data-agent-name]`)?.textContent;
const field = () => host.querySelector<HTMLInputElement>('[data-agent-naming] input');
const type = (el: HTMLInputElement, value: string) => act(() => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
});
const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); });

describe('naming an agent in the list', () => {
  it('the pencil opens a field holding the name; Enter keeps the new one everywhere the list names it', () => {
    render();
    const pencil = host.querySelector<HTMLButtonElement>('[data-agent-rename="laptop/a1"]') as HTMLButtonElement;
    expect(pencil.getAttribute('aria-label')).toBe('Rename claude · amber');
    act(() => pencil.click());
    const input = field() as HTMLInputElement;
    expect(input.value).toBe('claude · amber');
    expect(host.querySelector('[data-agent-rename="laptop/a1"]')).toBeNull(); // no pencil while naming
    type(input, '  Backend   fixer ');
    key(input, 'Enter');
    expect(field()).toBeNull();
    expect(nameOf('laptop/a1')).toBe('Backend fixer');
    expect(agentNameOf('laptop/a1')).toBe('Backend fixer');
    // The row's tooltip still says which agent it is.
    expect(host.querySelector('[data-agent-row="laptop/a1"]')?.getAttribute('title')).toContain('Backend fixer (claude · amber)');
    expect(nameOf('laptop/a2')).toBe('codex · teal');
  });

  it('F2 on a row starts naming; Escape keeps the old name and the blur after it changes nothing', () => {
    render();
    key(host.querySelector('[data-agent-row="laptop/a2"]') as Element, 'F2');
    const input = field() as HTMLInputElement;
    type(input, 'Reviewer');
    key(input, 'Escape');
    act(() => input.dispatchEvent(new FocusEvent('blur')));
    expect(nameOf('laptop/a2')).toBe('codex · teal');
    expect(agentNameOf('laptop/a2')).toBeUndefined();
  });

  it('leaving the field keeps the name; an empty name gives back the automatic one', () => {
    render();
    act(() => (host.querySelector('[data-agent-rename="laptop/a1"]') as HTMLButtonElement).click());
    type(field() as HTMLInputElement, 'Docs');
    act(() => (field() as HTMLInputElement).blur());
    expect(nameOf('laptop/a1')).toBe('Docs');
    act(() => (host.querySelector('[data-agent-rename="laptop/a1"]') as HTMLButtonElement).click());
    expect(field()?.placeholder).toBe('claude · amber');
    type(field() as HTMLInputElement, '   ');
    key(field() as HTMLInputElement, 'Enter');
    expect(nameOf('laptop/a1')).toBe('claude · amber');
    expect(localStorage.getItem('agora.agentNames.v1')).toBeNull();
  });
});

describe('the kept names', () => {
  it('survive a reload, and a stored value that is not ours is ignored', () => {
    setAgentName('n/a', 'One');
    resetAgentNames();
    expect(agentNameOf('n/a')).toBe('One');
    expect(parseAgentNames('nope').size).toBe(0);
    expect(parseAgentNames(JSON.stringify({ v: 2, names: { x: { name: 'y', at: 1 } } })).size).toBe(0);
    expect([...parseAgentNames(JSON.stringify({ v: 1, names: { x: { name: 'a\u0000b‮', at: 1 }, y: { name: 3 } } })).keys()]).toEqual(['x']);
  });

  it('are one line of at most 40 characters, and the oldest go first past the cap', () => {
    expect(cleanAgentName('a\nb')).toBe('a b');
    expect([...cleanAgentName('x'.repeat(60))].length).toBe(40);
    for (let i = 0; i <= MAX_AGENT_NAMES; i++) setAgentName(`n/${i}`, `A${i}`, localStorage, i + 1);
    resetAgentNames();
    expect(agentNameOf('n/0')).toBeUndefined();
    expect(agentNameOf(`n/${MAX_AGENT_NAMES}`)).toBe(`A${MAX_AGENT_NAMES}`);
  });
});

it('the hub window names an agent, and a helper it started, by the name the person gave it', () => {
  const v = fromSnapshot(agora.v1.Snapshot.create({ pos: 1, nodes: [{ nodeId: 'n', online: true }], agents: [
    { agentId: 'n/a', nodeId: 'n', kind: 'claude-code', running: true },
    { agentId: 'n/b', nodeId: 'n', kind: 'codex', running: true, parentAgentId: 'n/a' },
  ] }));
  const auto = tiles(v).find((t) => t.id === 'n/a')?.name as string;
  setAgentName('n/a', 'Lead');
  const list = tiles(v);
  expect(list.find((t) => t.id === 'n/a')).toMatchObject({ name: 'Lead', autoName: auto });
  expect(list.find((t) => t.id === 'n/b')).toMatchObject({ parentName: 'Lead' });
  expect(list.find((t) => t.id === 'n/b')?.autoName).toBeUndefined();
});
