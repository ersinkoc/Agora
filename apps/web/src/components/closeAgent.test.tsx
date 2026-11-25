// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel, SessionModel } from '../model';
import { closeAsk, closeReasonKey, isCloseCommand, losesWork } from '../net/closeAgent';
import { ChatActionsContext, type ChatActions } from './chatActions';
import { CloseAgentContext, CloseAgentMenu, CloseConfirm } from './CloseAgent';
import { PromptBox } from './PromptBox';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); closeAsk.cancel(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); closeAsk.cancel(); document.body.replaceChildren(); });

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n1/a-1', machineId: 'n1', name: 'one', accent: '#fff', machine: 'm', workspace: '/w', state: 'idle', preview: '', terminal: false, ...over });
const session = (names: string[]): SessionModel => ({ modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 0, contextWindow: 0, commands: names.map((name) => ({ name, description: 'd', inputHint: '' })) });
const flush = () => act(async () => { await Promise.resolve(); });
const click = (el: Element | null | undefined) => { if (!el) throw new Error('missing element'); act(() => { (el as HTMLElement).click(); }); };
const byText = (text: string, scope: ParentNode = document) => [...scope.querySelectorAll('button,[role=menuitem]')].find((b) => b.textContent?.trim() === text);
// The confirm is a dialog over the window (#1070), drawn in a portal: looked up in the document.
const confirmBox = () => document.querySelector('[role=alertdialog]') as HTMLElement;
const confirmTitle = () => confirmBox()?.querySelector('h2')?.textContent;

// The header menu and the confirm, for several agents on one page, like the agent list.
function Rows({ agents }: { agents: AgentTileModel[] }) {
  return <>{agents.map((a) => <div key={a.id} data-agent={a.id}><CloseAgentMenu agent={a} where="row" /><CloseConfirm agent={a} where="row" /></div>)}</>;
}
const withClose = (close: (a: AgentTileModel) => Promise<string | undefined>, node: React.ReactNode) => <CloseAgentContext.Provider value={close}>{node}</CloseAgentContext.Provider>;
const openMenu = (id: string) => click(host.querySelector(`[data-agent="${id}"] button[aria-haspopup=menu]`));

describe('Close agent: the confirm', () => {
  it('asks in a dialog, says what happens, and one click closes an idle agent', async () => {
    const close = vi.fn(async () => undefined);
    act(() => root.render(withClose(close, <Rows agents={[agent()]} />)));
    expect(confirmBox()).toBeNull();
    openMenu('n1/a-1');
    click(byText(t('close-agent')));
    expect(confirmBox().textContent).toContain(t('close-body'));
    expect(confirmBox().textContent).not.toContain(t('close-working'));
    expect(close).not.toHaveBeenCalled();
    click(byText(t('close-agent'), confirmBox()));
    await flush();
    expect(close).toHaveBeenCalledTimes(1);
    expect(confirmBox()).toBeNull();
  });

  it('makes agent names visible in the actions menu and confirm title', () => {
    const raw = 'clau\u3164de\u202e\ue000';
    act(() => root.render(withClose(async () => undefined, <Rows agents={[agent({ name: raw })]} />)));
    const trigger = host.querySelector<HTMLButtonElement>('[aria-haspopup=menu]')!;
    expect(trigger.getAttribute('aria-label')).toContain('clau\\u{3164}de\\u{202e}\\u{e000}');
    openMenu('n1/a-1'); click(byText(t('close-agent')));
    expect(confirmTitle()).toContain('clau\\u{3164}de\\u{202e}\\u{e000}');
  });

  it('verifies visible close labels while closing the raw agent identity', async () => {
    const close = vi.fn(async () => undefined);
    for (const [char, escaped] of [['\u2066', '\\u{2066}'], ['\u0001', '\\u{1}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const shown = agent({ id: `raw${char}id`, name: `co${char}dex` });
      act(() => root.render(withClose(close, <Rows agents={[shown]} />)));
      const trigger = host.querySelector<HTMLButtonElement>('[aria-haspopup=menu]')!;
      expect(trigger.getAttribute('aria-label')).toContain(`co${escaped}dex`);
      openMenu(shown.id); click(byText(t('close-agent')));
      expect(confirmTitle()).toContain(`co${escaped}dex`);
      click(byText(t('close-agent'), confirmBox()));
      await flush();
      expect(close).toHaveBeenLastCalledWith(shown);
    }
  });

  it('warns that the turn will be lost when the agent is working or waits for an answer', () => {
    for (const state of ['working', 'needs-you'] as const) {
      closeAsk.cancel();
      act(() => root.render(withClose(async () => undefined, <Rows agents={[agent({ state })]} />)));
      openMenu('n1/a-1'); click(byText(t('close-agent')));
      expect(document.querySelector('[data-close-warning]')?.textContent).toBe(t('close-working'));
    }
    expect(losesWork(agent({ state: 'idle' }))).toBe(false);
  });

  it('Keep it running closes the confirm and sends nothing', () => {
    const close = vi.fn(async () => undefined);
    act(() => root.render(withClose(close, <Rows agents={[agent()]} />)));
    openMenu('n1/a-1'); click(byText(t('close-agent')));
    click(byText(t('close-keep')));
    expect(confirmBox()).toBeNull();
    expect(close).not.toHaveBeenCalled();
  });

  it('closes exactly the agent whose item was clicked, never another one', async () => {
    const close = vi.fn(async (_: AgentTileModel) => undefined);
    const three = [agent({ id: 'n1/a-1', name: 'one' }), agent({ id: 'n2/b-2', machineId: 'n2', name: 'two' }), agent({ id: 'n1/c-3', name: 'three' })];
    act(() => root.render(withClose(close, <Rows agents={three} />)));
    openMenu('n2/b-2'); click(byText(t('close-agent')));
    expect(document.querySelectorAll('[role=alertdialog]').length).toBe(1);
    expect(confirmTitle()).toBe(t('close-title', { name: 'two' }));
    click(byText(t('close-agent'), confirmBox()));
    await flush();
    expect(close.mock.calls.map((c) => c[0].id)).toEqual(['n2/b-2']);
    // Asking for one agent and then another moves the confirm; it never keeps the first target.
    act(() => { closeAsk.ask('n1/a-1', 'row'); });
    act(() => { closeAsk.ask('n1/c-3', 'row'); });
    expect(document.querySelectorAll('[role=alertdialog]').length).toBe(1);
    expect(confirmTitle()).toBe(t('close-title', { name: 'three' }));
  });

  it('shows the localized reason when the command is refused, and keeps the agent', async () => {
    const close = vi.fn(async () => 'node.policy_denied');
    act(() => root.render(withClose(close, <Rows agents={[agent()]} />)));
    openMenu('n1/a-1'); click(byText(t('close-agent')));
    click(byText(t('close-agent'), confirmBox()));
    await flush();
    expect(confirmBox().querySelector('[role=alert]')?.textContent).toBe(t('close-failed-prefix', { reason: t('close-failed-policy') }));
  });

  it('has words in English and Turkish for every code it names', () => {
    for (const lang of ['en', 'tr'] as const) {
      setLanguage(lang);
      for (const code of ['auth.forbidden', 'node.policy_denied', 'approval.grant_required', 'approval.signature_invalid', 'node.offline', 'node.timeout', 'agent.not_found', 'protocol.capability_missing', 'hub.unreachable', 'x']) {
        const key = closeReasonKey(code);
        expect(t(key, { reason: 'r' }), `${lang} ${key}`).not.toBe(key);
      }
    }
  });

  it('is not offered without a hub, and not for a closed agent', () => {
    act(() => root.render(<Rows agents={[agent()]} />));
    expect(host.querySelector('button[aria-haspopup=menu]')).toBeNull();
    act(() => root.render(withClose(async () => undefined, <Rows agents={[agent({ closed: true })]} />)));
    expect(host.querySelector('button[aria-haspopup=menu]')).toBeNull();
  });

  it('the confirm goes away when the agent has closed meanwhile, and does not come back with its next run', () => {
    act(() => root.render(withClose(async () => undefined, <Rows agents={[agent()]} />)));
    openMenu('n1/a-1'); click(byText(t('close-agent')));
    act(() => root.render(withClose(async () => undefined, <Rows agents={[agent({ closed: true })]} />)));
    expect(closeAsk.get()).toBeUndefined();
    act(() => root.render(withClose(async () => undefined, <Rows agents={[agent()]} />)));
    expect(confirmBox()).toBeNull();
  });
});

describe('Close agent is not Stop turn', () => {
  const view = (a: AgentTileModel, close: () => Promise<string | undefined>, actions: ChatActions) =>
    withClose(close, <ChatActionsContext.Provider value={actions}><CloseConfirm agent={a} where="pane" /><PromptBox agent={a} send={async () => true} /></ChatActionsContext.Provider>);

  it('Stop turn only cancels the turn; Close agent only stops the agent', async () => {
    const close = vi.fn(async () => undefined);
    const cancelTurn = vi.fn(async () => true as const);
    const a = agent({ state: 'working', session: session(['compact']) });
    act(() => root.render(view(a, close, { cancelTurn })));
    click(byText(t('turn-stop')));
    await flush();
    expect(cancelTurn).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();
    expect(confirmBox()).toBeNull();
    act(() => { closeAsk.ask(a.id, 'pane'); });
    click(byText(t('close-agent'), confirmBox()));
    await flush();
    expect(close).toHaveBeenCalledTimes(1);
    expect(cancelTurn).toHaveBeenCalledTimes(1);
    expect(t('close-agent')).not.toBe(t('turn-stop'));
  });
});

describe('/exit and /quit', () => {
  const type = (value: string) => {
    const box = host.querySelector('textarea') as HTMLTextAreaElement;
    act(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(box, value); box.dispatchEvent(new Event('input', { bubbles: true })); });
    return box;
  };
  const enter = (box: HTMLElement) => act(() => { box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })); });

  it('is decided by the text alone, and the agent keeps a command of the same name', () => {
    expect(isCloseCommand('/exit', [])).toBe(true);
    expect(isCloseCommand(' /QUIT ', [])).toBe(true);
    expect(isCloseCommand('/exit now', [])).toBe(false);
    expect(isCloseCommand('please /exit', [])).toBe(false);
    expect(isCloseCommand('/exit', [{ name: 'exit', description: '', inputHint: '' }])).toBe(false);
    expect(isCloseCommand('/quit', [{ name: '/Quit', description: '', inputHint: '' }])).toBe(false);
    expect(isCloseCommand('/exit', [{ name: 'quit', description: '', inputHint: '' }])).toBe(true);
  });

  it('opens the confirm for this agent and is never sent as a message', async () => {
    const send = vi.fn(async () => true);
    const close = vi.fn(async () => undefined);
    const a = agent();
    act(() => root.render(withClose(close, <><CloseConfirm agent={a} where="pane" /><PromptBox agent={a} send={send} /></>)));
    enter(type('/exit'));
    await flush();
    expect(send).not.toHaveBeenCalled();
    expect(close).not.toHaveBeenCalled();
    expect(closeAsk.get()).toEqual({ id: 'n1/a-1', where: 'pane' });
    expect(confirmBox()).not.toBeNull();
    expect((host.querySelector('textarea') as HTMLTextAreaElement).value).toBe('');
  });

  it('opens the confirm even while the "/" menu lists other commands that start the same way', async () => {
    const send = vi.fn(async () => true);
    const a = agent({ session: session(['exit-plan']) });
    act(() => root.render(withClose(async () => undefined, <PromptBox agent={a} send={send} />)));
    enter(type('/exit'));
    await flush();
    expect(send).not.toHaveBeenCalled();
    expect(closeAsk.get()?.id).toBe('n1/a-1');
  });

  it('is sent to an agent that offers its own /exit, and does not open the confirm', async () => {
    const send = vi.fn(async () => true);
    const a = agent({ session: session(['exit']) });
    act(() => root.render(withClose(async () => undefined, <PromptBox agent={a} send={send} />)));
    const box = type('/exit ');
    enter(box);
    await flush();
    expect(send).toHaveBeenCalledWith(a, '/exit');
    expect(closeAsk.get()).toBeUndefined();
  });

  it('without a hub it is an ordinary message', async () => {
    const send = vi.fn(async () => true);
    act(() => root.render(<PromptBox agent={agent()} send={send} />));
    enter(type('/exit'));
    await flush();
    expect(send).toHaveBeenCalledTimes(1);
  });
});
