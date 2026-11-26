// @vitest-environment jsdom
// What the window says is on screen (net/onScreen.ts), which the sounds (never for the focused agent) and the finished mark (cleared once
// seen) read: the agents whose own view is in a visible pane, and the focused pane's agent; not a background tab, not a tool, not Home.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { workspaceOf } from '../layout/engine';
import { forgetLoaded, saveWorkspace } from '../layout/store';
import { LAYOUT_VERSION, type LayoutNode } from '../layout/tree';
import { onScreen } from '../net/onScreen';
import { demoSource } from '../terminal/source';
import { Console } from './Console';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => null }));

const agent = (id: string): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: 'var(--agent-teal)', kind: 'claude', machine: 'laptop', machineId: 'n1', workspace: 'w', state: 'working', preview: '', terminal: true });
const agents = ['n1/a', 'n1/b', 'n1/c', 'n1/d'].map(agent);

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); forgetLoaded(); });

function drawWith(rootNode: LayoutNode, focus: string) {
  forgetLoaded();
  saveWorkspace({ ws: { ...workspaceOf({ version: LAYOUT_VERSION, root: rootNode }), focus }, touched: true });
  act(() => root.render(<Console agents={agents} machines={[]} approvals={[]} messages={[]} terminalFor={() => demoSource('x')} onOpen={() => {}} link="live" />));
}

describe('what the window says is on screen', () => {
  it('is the agents in visible panes and the focused one, not a background tab nor a tool pane', () => {
    drawWith({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [
      { type: 'pane', id: 'p1', panel: 'agent', bind: { agent: 'n1/a' } },
      { type: 'tabs', active: 0, children: [{ type: 'pane', id: 'p2', panel: 'agent', bind: { agent: 'n1/b' } }, { type: 'pane', id: 'p3', panel: 'agent', bind: { agent: 'n1/c' } }] },
      { type: 'pane', id: 'p4', panel: 'changes', bind: { agent: 'n1/d' } },
    ] }, 'p1');
    expect(onScreen.get().focused).toBe('n1/a');
    expect([...onScreen.get().visible].sort()).toEqual(['n1/a', 'n1/b']);
  });

  it('has no focused agent when the focused pane is Home, and nothing once the window is gone', () => {
    drawWith({ type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [
      { type: 'pane', id: 'p1', panel: 'fleet' }, { type: 'pane', id: 'p2', panel: 'terminal', bind: { agent: 'n1/b' } },
    ] }, 'p1');
    expect(onScreen.get().focused).toBeUndefined();
    expect([...onScreen.get().visible]).toEqual(['n1/b']);
    act(() => root.unmount());
    root = createRoot(host);
    expect(onScreen.get().visible.size).toBe(0);
  });
});
