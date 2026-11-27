// @vitest-environment jsdom
// ADR-0119: a start another machine asks for shows the asking machine and its agent, says the folder is on this machine, and "More like
// this" is a standing grant of 6 (its helpers may ask too, when ticked), decided over v4 by the window.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { startRequest, type ApprovalModel } from '../net/view';
import { REMOTE_GRANT_SIZE, StartApproval, type StartChoice } from './StartApproval';

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

const detail = JSON.stringify({
  node: 'blackview', origin_node: 'mac', origin_agent: 'agent-3', origin_agent_kind: 'claude-code', kind: 'codex',
  folder: '/home/dev/code/login-fix', worktree: false, depth: 0, root_node: 'mac', root_agent: 'agent-3', grant_allowed: true,
  prompt: 'Write the tests.',
});
const approval = {
  id: 'blackview/ap-1', node: 'blackview', localId: 'ap-1', agentId: '', agentName: '', machineName: 'blackview',
  action: 'agent.start.remote', detail, expiresAtMs: 1, start: startRequest('agent.start.remote', detail),
} as ApprovalModel;

describe('a start another machine asks for', () => {
  it('is read with the asking machine and agent, and only that action', () => {
    const s = startRequest('agent.start.remote', detail);
    expect(s?.remote).toEqual({ originNode: 'mac', originAgent: 'agent-3', originAgentKind: 'claude-code' });
    expect(s?.inside).toBe(true);
    expect(s?.depth).toBe(0);
    expect(startRequest('agent.start', detail)?.remote).toBeUndefined();
  });

  it('names both machines, says where the folder is, and "More like this" is a standing grant of 6', () => {
    const answers: [boolean, StartChoice][] = [];
    act(() => root.render(<StartApproval approval={approval} start={approval.start!} onResolve={(ok, c) => answers.push([ok, c])} />));
    const text = host.textContent ?? '';
    expect(text).toContain('claude-code on mac asks to start a codex agent here on blackview');
    expect(host.querySelector('[data-remote-start]')?.textContent).toContain('the folder is on this machine');
    expect(text).toContain(`up to ${REMOTE_GRANT_SIZE} codex agents at a time, in this folder, until the run that asked ends (24 hours at most)`);
    const more = [...host.querySelectorAll<HTMLButtonElement>('[role=radio]')].find((b) => b.textContent === 'More like this');
    act(() => { more!.click(); });
    const tick = host.querySelector<HTMLInputElement>('[data-grant-choice] input[type=checkbox]');
    act(() => { tick!.click(); });
    const allow = [...host.querySelectorAll('button')].find((b) => b.textContent === 'Allow');
    act(() => { allow!.click(); });
    expect(answers).toEqual([[true, { grant: REMOTE_GRANT_SIZE, nested: true, remote: { anyKind: false, belowFolder: false, hours: 0 } }]]);
  });

  // ADR-0119 decision 12: the rest of the grant's scope is chosen on the card, said in full under it, and sent with the answer.
  it('lets the person pick any kind, the folders below and a time, and says so', () => {
    const answers: [boolean, StartChoice][] = [];
    act(() => root.render(<StartApproval approval={approval} start={approval.start!} onResolve={(ok, c) => answers.push([ok, c])} />));
    const more = [...host.querySelectorAll<HTMLButtonElement>('[role=radio]')].find((b) => b.textContent === 'More like this');
    act(() => { more!.click(); });
    act(() => { host.querySelector<HTMLInputElement>('[data-remote-any-kind]')!.click(); });
    act(() => { host.querySelector<HTMLInputElement>('[data-remote-below]')!.click(); });
    const eight = [...host.querySelectorAll<HTMLButtonElement>('[role=radio]')].find((b) => b.textContent === '8 hours');
    act(() => { eight!.click(); });
    expect(host.querySelector('[data-grant-more]')?.textContent).toContain(
      `up to ${REMOTE_GRANT_SIZE} agents of any kind this machine runs at a time, in this folder or any folder below it, for 8 hours`);
    const allow = [...host.querySelectorAll('button')].find((b) => b.textContent === 'Allow');
    act(() => { allow!.click(); });
    expect(answers).toEqual([[true, { grant: REMOTE_GRANT_SIZE, nested: false, remote: { anyKind: true, belowFolder: true, hours: 8 } }]]);
  });

  it('sends no scope when the answer is this one start only', () => {
    const answers: [boolean, StartChoice][] = [];
    act(() => root.render(<StartApproval approval={approval} start={approval.start!} onResolve={(ok, c) => answers.push([ok, c])} />));
    // With "Only this one" the scope's boxes are shut: a ticked box that changes nothing would read as a promise.
    for (const sel of ['[data-remote-any-kind]', '[data-remote-below]', '[data-grant-choice] input[type=checkbox]']) {
      expect(host.querySelector<HTMLInputElement>(sel)?.disabled, sel).toBe(true);
    }
    const allow = [...host.querySelectorAll('button')].find((b) => b.textContent === 'Allow');
    act(() => { allow!.click(); });
    expect(answers).toEqual([[true, { grant: 0, nested: false }]]);
  });
});

describe('a helper started from another machine', () => {
  it('hangs under its parent there in the agent tree', async () => {
    const { applyEvent, emptyView, tiles } = await import('../net/view');
    const { helperTree } = await import('../model');
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'mac', body: 'agentStarted', agentStarted: { agentId: 'agent-3', nodeId: 'mac', kind: 'claude-code' } } as never);
    applyEvent(v, 2, { producerId: 'blackview', body: 'agentStarted', agentStarted: { agentId: 'agent-7', nodeId: 'blackview', kind: 'codex', parentNodeId: 'mac', parentNodeAgentId: 'agent-3' } } as never);
    const tree = helperTree(tiles(v)).map(({ agent, depth }) => [agent.id, depth]);
    expect(tree).toEqual([['mac/agent-3', 0], ['blackview/agent-7', 1]]);
  });
});
