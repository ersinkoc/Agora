// @vitest-environment jsdom
// The grant question while a task waits for it (ADR-0041 with ADR-0095): what the owner sees must match what happened.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { forgetGiveSupport, giveTask, type GiveTask } from '../net/give';
import type { CommandBody } from '../net/live';
import { useGrantFlow, type GrantDeps } from '../net/useGrantFlow';
import { GiveDialog, type GiveApi } from './GiveTask';
import { GrantDialog } from './GrantDialog';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); forgetGiveSupport(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const agent: AgentTileModel = { id: 'n1/a-1', machineId: 'n1', name: 'claude · a-1', kind: 'claude', accent: '#fff', machine: 'laptop', workspace: '/w', state: 'idle', preview: '', projectId: 'p' };
const task: GiveTask = { key: 'p/t-1', projectId: 'p', taskId: 't-1', project: 'shop', title: 'Write the release notes' };
const flush = () => act(async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); });
const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text) as HTMLButtonElement | undefined;
const give = () => document.querySelector('[data-give-dialog]') as HTMLElement;

// What App does: the live connection reports the refusal to the grant flow, the Give dialog sends through the same command.
let flow: ReturnType<typeof useGrantFlow>;
function Harness({ command, sign }: { command: (node: string, body: CommandBody) => Promise<object>; sign: GrantDeps['sign'] }) {
  const wrapped = async (node: string, body: CommandBody) => {
    try { return await command(node, body); } catch (e) { if ((e as { code?: string }).code === 'approval.grant_required') flow.request(node, body); throw e; }
  };
  flow = useGrantFlow({ command: wrapped, sign });
  const api: GiveApi = { agents: [agent], machines: [{ id: 'n1', name: 'laptop' }], send: (a, t) => giveTask(wrapped, a, t) };
  return <>
    <GiveDialog task={task} give={api} onClose={() => {}} />
    {flow.question && <GrantDialog machine="laptop" agent="claude · a-1" kind={flow.question.ask.kind} busy={flow.question.busy} failed={flow.question.failed} onAllow={() => void flow.allow()} onClose={flow.dismiss} />}
  </>;
}

function setup() {
  let dispatches = 0;
  const command = vi.fn(async (_node: string, body: CommandBody) => {
    if (body.grantCreate) return {};
    dispatches++;
    if (dispatches === 1) throw { code: 'approval.grant_required', params: {} };
    return { taskDispatch: { queued: false } };
  });
  let signed!: () => void;
  const sign = vi.fn(() => new Promise<{ grantId: string }>((resolve) => { signed = () => resolve({ grantId: 'gr-1' }); }));
  act(() => root.render(<Harness command={command} sign={sign} />));
  act(() => { give().querySelector<HTMLButtonElement>('[data-agent-id] button')!.click(); });
  act(() => { button('Give the task')!.click(); });
  return { command, sign, sign_done: () => signed() };
}
const dispatchCalls = (command: ReturnType<typeof vi.fn>) => command.mock.calls.filter(([, b]) => (b as CommandBody).taskDispatch).length;

describe('the grant question for a task', () => {
  it('ignores Not now, Escape and a click outside while Allow is signing, and then shows that the task was sent', async () => {
    const s = setup();
    await flush();
    expect(give().textContent).toContain('This machine needs your permission first.');
    act(() => { button('Allow for 1 hour')!.click(); });
    expect(button('Allowing…')?.disabled).toBe(true);
    expect(button('Not now')?.disabled).toBe(true);
    // Every way to dismiss it while the signature is being made.
    act(() => { button('Not now')!.click(); });
    act(() => { document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    act(() => { document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); });
    act(() => { flow.dismiss(); });
    expect(button('Allowing…')).toBeDefined();
    expect(give().textContent).not.toContain('nothing was sent');
    await act(async () => { s.sign_done(); for (let i = 0; i < 6; i++) await Promise.resolve(); });
    await flush();
    expect(dispatchCalls(s.command)).toBe(2);
    expect(button('Allowing…')).toBeUndefined();
    expect(give().textContent).toContain('Sent to claude · a-1.');
  });

  it('says nothing was sent when the owner says Not now, and sends nothing', async () => {
    const s = setup();
    await flush();
    act(() => { button('Not now')!.click(); });
    await flush();
    expect(give().textContent).toContain('nothing was sent. You can try again.');
    expect(dispatchCalls(s.command)).toBe(1);
    expect(s.sign).not.toHaveBeenCalled();
    expect(button('Allow for 1 hour')).toBeUndefined();
  });
});

// A refused terminal or chat subscription asks for that agent only, and the window subscribes again on its own stream.
function WatchHarness({ command, sign }: { command: GrantDeps['command']; sign: GrantDeps['sign'] }) {
  flow = useGrantFlow({ command, sign });
  return flow.question ? <GrantDialog machine="laptop" agent="claude · a-1" kind={flow.question.ask.kind} busy={flow.question.busy} failed={flow.question.failed} onAllow={() => void flow.allow()} onClose={flow.dismiss} /> : null;
}

describe('the grant question for watching', () => {
  for (const body of [{ terminalSubscribe: { terminalId: 't-a-1' } }, { chatSubscribe: { agentId: 'a-1' } }] as CommandBody[]) {
    it(`asks to watch that agent and does not resend ${Object.keys(body)[0]}`, async () => {
      const command = vi.fn(async (_node: string, _body: CommandBody) => ({}));
      const sign = vi.fn(async (_node: string, ask: { scope: string; commands: string[] }) => ({ grantId: 'gr-1', ...ask }));
      act(() => root.render(<WatchHarness command={command} sign={sign as unknown as GrantDeps['sign']} />));
      act(() => flow.request('n1', body));
      expect(document.body.textContent).toContain('Allow watching claude · a-1 for 1 hour?');
      await act(async () => { button('Allow for 1 hour')!.click(); for (let i = 0; i < 6; i++) await Promise.resolve(); });
      await flush();
      expect(sign.mock.calls[0]?.[1].scope).toBe('agent:a-1');
      expect(command.mock.calls.map(([, b]) => Object.keys(b)[0])).toEqual(['grantCreate']);
    });
  }
});
