// @vitest-environment jsdom
// The panel registry (ADR-0068 rung 3, docs/ux/layout.md "Panels"): every panel of the layout's table is registered with its facts and
// its drawing, a pane's panel is drawn by a lookup, an id nobody registered is the "unknown panel" placeholder, a panel's drawing gets
// only its context, and the window (not the panel) draws the pane bar, where ADR-0114 puts a plugin's marker later.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, useContext, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { AnswerGuard } from '../components/useAnswerGuard';
import { CloseAgentContext } from '../components/CloseAgent';
import { ChatActionsContext, type ChatActions } from '../components/chatActions';
import { GiveContext, type GiveApi } from '../components/GiveTask';
import type { TerminalSource } from '../terminal/source';
import { BUILTIN_REGISTRY, type HostActions, withPaths } from './builtinPanels';
import { type Action, type Workspace as WorkspaceState, reduce, workspaceOf } from './engine';
import { BUILTIN_PANELS, PANEL_INFO, TOOL_PANELS, isToolPanel, panelRegistry, type PanelContext, type PanelDef, type PanelState } from './panels';
import { streamPlan } from './streams';
import { LAYOUT_VERSION, type LayoutNode } from './tree';
import { Workspace } from './Workspace';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); document.body.querySelectorAll('[role=menu]').forEach((m) => m.remove()); });

const agent = (id: string, name: string): AgentTileModel => ({ id, name, accent: 'var(--agent-teal)', machine: 'Laptop', machineId: 'laptop', workspace: 'proj', project: 'proj', state: 'working', preview: '' });
const fleet = [agent('n/a', 'claude · amber'), agent('n/b', 'codex · teal')];
const state: PanelState = Object.freeze({ fleet, machines: [], approvals: [], attention: [], changed: [], messages: [], boards: [], projects: [], now: 0, link: 'live' as const });
const none = () => {};
const actions: HostActions = Object.freeze({ openAgent: none, openFromHome: none, openTool: none, review: none, history: none, browse: none, showNeeds: none, showChanges: none, dismiss: none, answers: {} as AnswerGuard, chat: {} });
const ws = (root: LayoutNode): WorkspaceState => workspaceOf({ version: LAYOUT_VERSION, root });
const pane = (id: string, panel: string, bind?: { agent: string }) => ({ type: 'pane' as const, id, panel, ...(bind ? { bind } : {}) });
const row = (...children: LayoutNode[]): LayoutNode => ({ type: 'split', direction: 'row', ratios: children.map(() => 1 / children.length), children });

// A window drawn with a given registry; terminals are counted per agent, and every dispatch is kept.
function draw(start: WorkspaceState, panels = BUILTIN_REGISTRY) {
  const terminals: string[] = [];
  const dispatched: Action[] = [];
  const terminalFor = (id: string): TerminalSource => { terminals.push(id); return { connect: () => ({ sendInput() {}, resize() {}, close() {} }) }; };
  function Harness() {
    const [cur, set] = useState(start);
    return <div style={{ position: 'relative', width: 800, height: 600 }}>
      <Workspace ws={cur} dispatch={(a) => { dispatched.push(a); set((w) => reduce(w, a)); }} fleet={fleet} bindings={{ attention: new Map() }} narrow={false}
        panels={panels} state={state} sources={{}} actions={actions} terminalFor={terminalFor} welcome={<p data-welcome>welcome</p>} />
    </div>;
  }
  act(() => root.render(<Harness />));
  return { terminals, dispatched };
}

describe('the registry', () => {
  it('has every panel of the layout table in docs/ux/layout.md, and no other', () => {
    const doc = readFileSync(join(__dirname, '../../../../docs/ux/layout.md'), 'utf8');
    const table = doc.slice(doc.indexOf('### Panels (version 1)'), doc.indexOf('## Reading a layout'));
    const ids = [...table.matchAll(/^\| `([a-z-]+)` \|/gm)].map((m) => m[1]);
    expect(ids.length).toBeGreaterThan(10);
    expect([...BUILTIN_REGISTRY.ids].sort()).toEqual([...ids].sort());
    expect(PANEL_INFO.ids).toEqual(BUILTIN_REGISTRY.ids);
  });

  it('looks panels up by id: kind, name, binding, streaming; an id it does not have is undefined', () => {
    expect(BUILTIN_REGISTRY.get('terminal')).toMatchObject({ kind: 'tool', binds: 'agent', streams: true, label: 'focus-terminal' });
    expect(BUILTIN_REGISTRY.get('approvals')).toMatchObject({ kind: 'global', binds: 'nothing', streams: false });
    expect(BUILTIN_REGISTRY.get('empty')?.kind).toBe('chooser');
    expect(BUILTIN_REGISTRY.get('machines-map')).toBeUndefined();
    expect(BUILTIN_REGISTRY.label('machines-map')).toBeUndefined();
    expect(BUILTIN_REGISTRY.streams('machines-map')).toBe(false);
    // Only the agent's own view and the terminal draw a live terminal; only Home and the empty pane stand alone without a bar.
    expect(BUILTIN_PANELS.filter((p) => p.streams).map((p) => p.id)).toEqual(['terminal', 'agent']);
    expect(BUILTIN_PANELS.filter((p) => 'lone' in p && p.lone).map((p) => p.id)).toEqual(['fleet', 'empty']);
    // Every panel but the chooser has its drawing.
    for (const id of BUILTIN_REGISTRY.ids) {
      const def = BUILTIN_REGISTRY.get(id) as PanelDef;
      expect(typeof def.render, id).toBe(def.kind === 'chooser' ? 'undefined' : 'function');
    }
  });

  it('lists what Show… and the chooser offer in the window\'s order, and the tools for the palette', () => {
    expect(BUILTIN_REGISTRY.offered('global').map((p) => p.id)).toEqual(['fleet', 'approvals', 'tasks', 'messages', 'usage', 'preview', 'theme']);
    expect(BUILTIN_REGISTRY.offered('tool').map((p) => p.id)).toEqual(['terminal', 'changes', 'files', 'manager']);
    expect(TOOL_PANELS).toEqual(['terminal', 'changes', 'files', 'manager']);
    expect(isToolPanel('manager') && !isToolPanel('agent') && !isToolPanel('approvals')).toBe(true);
  });

  it('keeps the first panel registered under an id, and is frozen', () => {
    const r = panelRegistry<PanelDef>([
      { id: 'x', label: 'fleet-panel', kind: 'global', binds: 'nothing', streams: false, render: () => 'first' },
      { id: 'x', label: 'board-title', kind: 'global', binds: 'nothing', streams: true, render: () => 'second' },
    ]);
    expect(r.ids).toEqual(['x']);
    expect(r.label('x')).toBe('fleet-panel');
    expect(Object.isFrozen(r) && Object.isFrozen(r.get('x')) && Object.isFrozen(r.ids)).toBe(true);
  });

  it('decides which panes stream: a registry whose agent view does not stream plans none', () => {
    const w = ws(row(pane('p1', 'agent', { agent: 'n/a' }), pane('p2', 'terminal', { agent: 'n/b' }), pane('p3', 'changes', { agent: 'n/b' })));
    const b = { attention: new Map<string, string>() };
    expect([...streamPlan(w, false, b, () => true)]).toEqual([['p1', 'n/a'], ['p2', 'n/b']]);
    const quiet = panelRegistry([{ id: 'agent', label: 'layout-panel-agent', kind: 'agent', binds: 'agent', streams: false }]);
    expect(streamPlan(w, false, b, () => true, quiet).size).toBe(0);
  });
});

describe('the window draws panels from the registry', () => {
  it('shows an id nobody registered as the visible "unknown panel" placeholder, under the window\'s bar, and keeps it in the layout', () => {
    const { dispatched } = draw(ws(row(pane('p1', 'machines-map'), pane('p2', 'browser'))));
    const body = host.querySelector('#pane-p1') as HTMLElement;
    expect(body.textContent).toBe('Unknown panel “machines-map”: this window cannot show it.');
    expect(host.querySelector('#pane-p2')?.textContent).toBe('Unknown panel “browser”: this window cannot show it.');
    expect(host.querySelectorAll('.pane-bar')).toHaveLength(2);
    expect([...host.querySelectorAll('[data-pane-name]')].map((n) => n.textContent)).toEqual(['Unknown panel “machines-map”: this window cannot show it.', 'Unknown panel “browser”: this window cannot show it.']);
    expect(dispatched.filter((a) => a.type !== 'focus')).toEqual([]);
  });

  it('shows the placeholder alone in the window too, with its bar (only Home and the empty pane go without one)', () => {
    draw(ws(pane('p1', 'machines-map')));
    expect(host.querySelector('.pane-bar')).not.toBeNull();
    expect(host.querySelector('#pane-p1')?.textContent).toContain('Unknown panel “machines-map”');
    draw(ws(pane('p1', 'empty')));
    expect(host.querySelector('.pane-bar')).toBeNull();
    expect(host.querySelector('[data-welcome]')).not.toBeNull();
  });

  it('shows the placeholder for a registered panel that cannot draw here', () => {
    const r = panelRegistry<PanelDef>([{ id: 'clock', label: 'fleet-panel', kind: 'global', binds: 'nothing', streams: false, render: () => undefined }]);
    draw(ws(row(pane('p1', 'clock'), pane('p2', 'clock'))), r);
    expect(host.querySelector('#pane-p1')?.textContent).toBe('Unknown panel “clock”: this window cannot show it.');
  });

  it('draws the pane bar itself: a panel draws only inside its own pane body, under the bar, even alone in the window', () => {
    let calls = 0;
    const r = panelRegistry<PanelDef>([{ id: 'clock', label: 'board-title', kind: 'global', binds: 'nothing', streams: false, render: () => { calls++; return <p data-plugin-body>ten past</p>; } }]);
    draw(ws(pane('p1', 'clock')), r);
    const bar = host.querySelector('.pane-bar') as HTMLElement;
    expect(bar).not.toBeNull();
    expect(bar.querySelector('[data-pane-name]')?.textContent).toBe('Shared tasks and files'); // the registered name, drawn by the window
    expect(bar.querySelector('[data-plugin-body]')).toBeNull();
    const drawn = host.querySelectorAll('[data-plugin-body]');
    expect(drawn).toHaveLength(1);
    expect(drawn[0]?.closest('[data-pane]')?.id).toBe('pane-p1');
    expect((drawn[0]?.parentElement as HTMLElement).id).toBe('pane-p1');
    expect(calls).toBeGreaterThan(0);
  });

  it('lists the registry in the empty pane\'s chooser, tools first, and in Show…, the panels first', () => {
    draw(ws(row(pane('p1', 'empty'), pane('p2', 'machines-map'))));
    const chips = [...(host.querySelector('#pane-p1') as HTMLElement).querySelectorAll('.ide-chip')].map((b) => b.textContent);
    expect(chips).toEqual(['What needs you next', 'The agent selected in the list', 'Terminal', 'Changes', 'Files', 'File manager', 'Fleet', 'Needs you', 'Shared tasks and files', 'Team chat', 'Usage', 'Preview', 'Theme']);
    act(() => { (host.querySelector('[aria-label="Pane options: Empty pane"]') as HTMLButtonElement).click(); });
    const show = [...document.body.querySelectorAll<HTMLElement>('[role=menuitem]')].find((m) => m.textContent?.startsWith('Show')) as HTMLElement;
    act(() => { show.click(); });
    const items = [...document.body.querySelectorAll<HTMLElement>('[role=menuitem], [role=menuitemcheckbox], [role=menuitemradio]')].map((m) => m.textContent?.trim());
    const from = items.indexOf('What needs you next');
    expect(items.slice(from)).toEqual(['What needs you next', 'The agent selected in the list', 'claude · amber', 'codex · teal',
      'Fleet', 'Needs you', 'Shared tasks and files', 'Team chat', 'Usage', 'Preview', 'Theme', 'Terminal', 'Changes', 'Files', 'File manager']);
  });
});

describe('a panel\'s drawing gets only its context', () => {
  function spy(id: string, kind: 'global' | 'tool' | 'agent', streams: boolean) {
    const seen: { args: unknown[]; ctx: PanelContext }[] = [];
    const r = panelRegistry<PanelDef>([{ id, label: 'board-title', kind, binds: kind === 'global' ? 'nothing' : 'agent', streams, render: (...args: unknown[]) => { seen.push({ args, ctx: args[0] as PanelContext }); return <p>drawn</p>; } }]);
    return { r, seen };
  }

  it('one frozen argument: its pane, panel, binding, agent, the read-only state and reads, its actions, and a terminal only when it streams', () => {
    const { r, seen } = spy('watch', 'agent', true);
    const { dispatched } = draw(ws(row(pane('p1', 'watch', { agent: 'n/a' }), pane('p2', 'nothing-here'))), r);
    const { args, ctx } = seen[seen.length - 1] as { args: unknown[]; ctx: PanelContext };
    expect(args).toHaveLength(1);
    expect(Object.keys(ctx).sort()).toEqual(['actions', 'agent', 'bind', 'pane', 'panel', 'sources', 'state', 'terminal']);
    expect(Object.isFrozen(ctx) && Object.isFrozen(ctx.actions) && Object.isFrozen(ctx.state)).toBe(true);
    expect(ctx).toMatchObject({ pane: 'p1', panel: 'watch', bind: { agent: 'n/a' } });
    expect(ctx.agent?.name).toBe('claude · amber');
    expect(ctx.state).toBe(state);
    // The actions are the window's own and this pane's rebind: no dispatch, no connection, no signing.
    expect(Object.keys(ctx.actions).sort()).toEqual([...Object.keys(actions), 'rebind'].sort());
    act(() => ctx.actions.rebind('n/b'));
    expect(dispatched.filter((a) => a.type === 'bind')).toEqual([{ type: 'bind', pane: 'p1', bind: { agent: 'n/b' }, panel: 'watch' }]);
    expect(typeof ctx.terminal).toBe('function');
  });

  it('a panel that does not stream gets no terminal, and one bound to nothing sees no agent even when the pane kept one', () => {
    const quiet = spy('watch', 'agent', false);
    draw(ws(row(pane('p1', 'watch', { agent: 'n/a' }), pane('p2', 'nothing-here'))), quiet.r);
    expect(quiet.seen.at(-1)?.ctx.terminal).toBeUndefined();
    act(() => root.unmount());
    root = createRoot(host);
    const global = spy('board', 'global', false);
    draw(ws(row(pane('p1', 'board', { agent: 'n/a' }), pane('p2', 'nothing-here'))), global.r);
    const ctx = global.seen.at(-1)?.ctx as PanelContext;
    expect(ctx.agent).toBeUndefined();
    expect(ctx.bind).toBeUndefined();
    expect(ctx.terminal).toBeUndefined();
  });
});

describe('a built-in panel sends commands only through its actions', () => {
  it('closes an agent only when its actions can, whatever the app provides around it', () => {
    const running = { ...agent('n/c', 'gemini · rose'), terminal: false };
    const appClose = async () => undefined;
    const ctx = (closeAgent?: PanelContext['actions']['closeAgent']): PanelContext => Object.freeze({
      pane: 'p1', panel: 'agent', bind: { agent: 'n/c' }, agent: running, state, sources: {}, terminal: undefined,
      actions: Object.freeze({ ...actions, rebind: none, ...(closeAgent ? { closeAgent } : {}) }),
    });
    const def = BUILTIN_REGISTRY.get('agent') as Extract<PanelDef, { render: unknown }>;
    const menu = () => host.querySelector('[aria-label="Actions for gemini · rose"]');
    act(() => root.render(<CloseAgentContext.Provider value={appClose}>{def.render(ctx())}</CloseAgentContext.Provider>));
    expect(host.querySelector('.agent-body')).not.toBeNull();
    expect(menu()).toBeNull();
    act(() => root.render(<CloseAgentContext.Provider value={undefined}>{def.render(ctx(appClose))}</CloseAgentContext.Provider>));
    expect(menu()).not.toBeNull();
  });
});

describe('a built-in panel\'s settings and give paths come from its actions', () => {
  it('gives the panel its actions\' chat and give, not what the app provides around it', () => {
    const seen: { chat: ChatActions; give?: GiveApi }[] = [];
    function Probe() { seen.push({ chat: useContext(ChatActionsContext), give: useContext(GiveContext) }); return null; }
    const chat: ChatActions = { cancelTurn: async () => true };
    const give = { agents: [], machines: [], send: async () => ({ ok: true }) } as unknown as GiveApi;
    const panelActions = Object.freeze({ ...actions, rebind: none, chat, give });
    act(() => root.render(<ChatActionsContext.Provider value={{ setConfig: async () => true }}><GiveContext.Provider value={undefined}>{withPaths(panelActions, <Probe />)}</GiveContext.Provider></ChatActionsContext.Provider>));
    expect(seen.at(-1)?.chat).toBe(chat);
    expect(seen.at(-1)?.give).toBe(give);
  });
});

describe('only panes on screen draw, so only they subscribe to a terminal', () => {
  it('a background tab and the panes behind a maximized one are never drawn and never get a terminal', () => {
    const drawn: string[] = [];
    const r = panelRegistry<PanelDef>([{ id: 'agent', label: 'layout-panel-agent', kind: 'agent', binds: 'agent', streams: true,
      render: (ctx) => { drawn.push(ctx.pane); ctx.terminal?.(); return <p>{ctx.agent?.name}</p>; } }]);
    const tabs = { type: 'tabs' as const, active: 0, children: [pane('p1', 'agent', { agent: 'n/a' }), pane('p2', 'agent', { agent: 'n/b' })] };
    const first = draw(ws(tabs), r);
    expect(new Set(drawn)).toEqual(new Set(['p1']));
    expect(new Set(first.terminals)).toEqual(new Set(['n/a']));

    act(() => root.unmount());
    root = createRoot(host);
    drawn.length = 0;
    const zoomed = reduce(ws(row(pane('p1', 'agent', { agent: 'n/a' }), pane('p2', 'agent', { agent: 'n/b' }))), { type: 'maximize', pane: 'p2' });
    const second = draw(zoomed, r);
    expect(new Set(drawn)).toEqual(new Set(['p2']));
    expect(new Set(second.terminals)).toEqual(new Set(['n/b']));
    expect(host.querySelector('#pane-p1')).toBeNull();
  });
});
