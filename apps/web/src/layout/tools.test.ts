import { beforeEach, describe, expect, it } from 'vitest';
import { type Workspace, reduce, workspaceOf } from './engine';
import { forgetLoaded, loadWorkspace, saveWorkspace } from './store';
import { MAX_PANES, bindOf, defaultLayout, isValid, panesOf, parseLayout, serialiseLayout } from './tree';
import { PANEL_INFO } from './panels';
import { subscribeToolRequests, requestTool, takeToolRequests } from './requests';

// Tool panes (Terminal, Changes, Files, File manager): panes bound to one agent, placed by the same layout machinery as any other.
const ws0 = (): Workspace => workspaceOf(defaultLayout());
const open = (ws: Workspace, panel: string, agent?: string) => reduce(ws, { type: 'show', panel, ...(agent ? { bind: { agent } } : {}) });
const kinds = (ws: Workspace) => panesOf(ws.layout.root).map((p) => `${p.panel}${bindOf(p) ? `:${bindOf(p)}` : ''}`);

beforeEach(() => forgetLoaded());

describe('opening a tool pane', () => {
  it('binds the only, empty pane when the window has nothing else', () => {
    expect(kinds(open(ws0(), 'changes', 'n/a'))).toEqual(['changes:n/a']);
  });
  it('puts it beside the pane in focus, which stays', () => {
    const ws = open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'files', 'n/a');
    expect(kinds(ws)).toEqual(['agent:n/a', 'files:n/a']);
    expect(ws.focus).toBe(panesOf(ws.layout.root)[1]?.id);
    expect(isValid(ws.layout)).toBe(true);
  });
  it('brings an existing pane of that tool for that agent to the front instead of making a second', () => {
    let ws = open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'changes', 'n/a');
    ws = reduce(ws, { type: 'focus', pane: panesOf(ws.layout.root)[0]?.id as string });
    const again = open(ws, 'changes', 'n/a');
    expect(kinds(again)).toEqual(kinds(ws));
    expect(again.focus).toBe(panesOf(ws.layout.root)[1]?.id);
  });
  it('turns a chooser of that tool that waits for an agent into the pane asked for (a new terminal leaves no empty chooser behind)', () => {
    let ws = open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'terminal'); // the chooser
    const chooser = panesOf(ws.layout.root).find((p) => p.panel === 'terminal' && !p.bind)?.id;
    ws = open(ws, 'terminal', 'n/new');
    expect(kinds(ws)).toEqual(['agent:n/a', 'terminal:n/new']);
    expect(panesOf(ws.layout.root)[1]?.id).toBe(chooser);
  });
  it('makes another for another agent (two folders side by side)', () => {
    let ws = open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'changes', 'n/a');
    ws = open(ws, 'changes', 'n/b');
    expect(kinds(ws).filter((k) => k.startsWith('changes')).sort()).toEqual(['changes:n/a', 'changes:n/b']);
  });
  it('without an agent, any pane of that tool is the one; none yet means an unbound pane that asks which folder', () => {
    const ws = open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'manager');
    expect(kinds(ws)).toEqual(['agent:n/a', 'manager']);
    expect(kinds(open(ws, 'manager'))).toEqual(kinds(ws));
  });
  it('can be split, moved and dragged like any pane, and pointed at another folder', () => {
    let ws = open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'files', 'n/a');
    const [a, f] = panesOf(ws.layout.root).map((p) => p.id) as [string, string];
    ws = reduce(ws, { type: 'split', pane: f, direction: 'column', panel: 'manager', bind: { agent: 'n/a' } });
    ws = reduce(ws, { type: 'move', pane: f, target: a, place: 'center' });
    ws = reduce(ws, { type: 'bind', pane: f, bind: { agent: 'n/z' }, panel: 'files' });
    expect(kinds(ws).sort()).toEqual(['agent:n/a', 'files:n/z', 'manager:n/a']);
    expect(isValid(ws.layout)).toBe(true);
  });
  it('refuses, as for any panel, when the window is full', () => {
    let ws = reduce(ws0(), { type: 'preset', preset: '3x2', agents: Array.from({ length: MAX_PANES }, (_, i) => `n/${i}`) });
    expect(panesOf(ws.layout.root).length).toBeLessThanOrEqual(MAX_PANES);
    ws = open(ws, 'terminal', 'n/new');
    expect(panesOf(ws.layout.root).length).toBeLessThanOrEqual(MAX_PANES);
    expect(isValid(ws.layout)).toBe(true);
  });
  it('closes with its agent: a pane of an agent that is gone does not stay', () => {
    const ws = reduce(open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'changes', 'n/a'), { type: 'prune', alive: new Set<string>() });
    expect(kinds(ws).some((k) => k.startsWith('changes:'))).toBe(false);
  });
});

describe('remembering the panes on this device', () => {
  const memory = () => { const m = new Map<string, string>(); return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); } }; };
  it('the window comes back with its tool panes, their folders and the one in focus', () => {
    const kv = memory();
    let ws = open(reduce(ws0(), { type: 'open', agent: 'n/a' }), 'changes', 'n/a');
    ws = open(ws, 'manager', 'n/a');
    ws = open(ws, 'terminal', 'n/b');
    saveWorkspace({ ws, touched: true }, kv);
    forgetLoaded();
    const back = loadWorkspace(kv);
    expect(back.ws).toEqual(ws);
    expect(kinds(back.ws)).toContain('manager:n/a');
  });
  it('is written in the layout file, which a client that does not know a panel shows as unknown rather than refusing', () => {
    expect(['terminal', 'changes', 'files', 'manager'].every((id) => PANEL_INFO.get(id)?.kind === 'tool')).toBe(true);
    const ws = open(ws0(), 'manager', 'n/a');
    const parsed = parseLayout(JSON.parse(serialiseLayout(ws.layout)));
    expect(parsed.ok && kinds(workspaceOf(parsed.layout))).toEqual(['manager:n/a']);
    const future = parseLayout({ version: 1, root: { type: 'pane', id: 'p1', panel: 'browser', bind: { agent: 'n/a' } } });
    expect(future.ok).toBe(true);
  });
});

describe('asking for a pane from outside the window', () => {
  it('holds requests until the window takes them, in order', () => {
    takeToolRequests();
    let calls = 0;
    const off = subscribeToolRequests(() => { calls++; });
    requestTool({ panel: 'changes' });
    requestTool({ panel: 'terminal', agent: 'n/a', wait: true });
    off();
    requestTool({ panel: 'files' });
    expect(calls).toBe(2);
    expect(takeToolRequests().map((r) => r.panel)).toEqual(['changes', 'terminal', 'files']);
    expect(takeToolRequests()).toEqual([]);
  });
});
