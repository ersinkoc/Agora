// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { setLanguage, t } from '../i18n';
import { endedWithError } from '../model';
import type { ChatLine, ChatSource } from '../net/chat';
import { fromSnapshot, tiles } from '../net/view';
import { ClosedSummary } from './ClosedSummary';
import { PromptBox } from './PromptBox';
import { refusalWords } from './StartAgent';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

// A chat agent the node gave up on, as a window that was reloaded gets it: from the hub's snapshot, closed with the exit code the node sent.
const reloaded = (exitCode: number) => {
  const v = fromSnapshot(agora.v1.Snapshot.create({ pos: 4, nodes: [{ nodeId: 'n', online: true }], agents: [{ agentId: 'n/agent-1', nodeId: 'n', kind: 'rm-chat', running: false, exitCode}] }));
  const a = tiles(v)[0];
  if (!a) throw new Error('no agent');
  return a;
};
const notice = (params: Record<string, string>): ChatLine => ({ id: 9, atMs: 1, role: 'agent', kind: 'notice', text: '', path: '', code: 'adapter.gave_up', params });
const sourceOf = (lines: ChatLine[]): ChatSource => ({ connect(on) { on(lines, true); return { close() {} }; } });

describe('a chat agent the node gave up on (#902)', () => {
  it('stays ended with an error after a reload and takes no more prompts', () => {
    // SIGKILL on every restart is sent as 137 (128 + 9), which reads as a failure, never as a plain end.
    const a = reloaded(137);
    expect(a.closed).toBe(true);
    expect(a.terminal).toBe(false);
    expect(endedWithError(a)).toBe(true);
    expect(endedWithError({ closed: true, exitCode: -9 })).toBe(false); // an ordinary agent ended by a stop is shown as before
    act(() => root.render(<PromptBox agent={a} send={async () => true} />));
    expect(host.querySelector('textarea')?.disabled).toBe(true);
    const sendButton = [...host.querySelectorAll('button[type=submit]')][0] as HTMLButtonElement | undefined;
    expect(sendButton?.disabled).toBe(true);
  });

  it('says why in its summary, with the exit code only when the node knew it', () => {
    act(() => root.render(<ClosedSummary agent={reloaded(3)} now={2} source={sourceOf([notice({ restarts: '5', exit_code: '3' })])} onDismiss={() => {}} />));
    expect(host.textContent).toContain(t('closed-failed', { code: 3 }));
    expect(host.querySelector('[data-gave-up]')?.textContent).toBe(t('notice-adapter-gave_up', { restarts: '5', exit_code: '3' }));

    // Not known: the wire carries 1 (never a success), and nothing says "exit code 1".
    act(() => root.render(<ClosedSummary agent={reloaded(1)} now={2} source={sourceOf([notice({ restarts: '5' })])} onDismiss={() => {}} />));
    expect(host.textContent).toContain(t('closed-failed-signal'));
    expect(host.textContent).not.toContain('exit code 1');
    expect(host.querySelector('[data-gave-up]')?.textContent).toBe(t('notice-adapter-gave_up-no_code', { restarts: '5' }));
    expect(host.querySelector('[data-gave-up]')?.textContent).toContain('5 restarts');

    // A closed agent without such a notice says nothing more than before.
    act(() => root.render(<ClosedSummary agent={reloaded(1)} now={2} source={sourceOf([])} onDismiss={() => {}} />));
    expect(host.querySelector('[data-gave-up]')).toBeNull();
    expect(host.textContent).toContain(t('closed-failed', { code: 1 }));
  });
});

describe('a start refused because the program is missing (#903)', () => {
  it('names the program in the words of the "not installed" note', () => {
    const [id, args] = refusalWords('start-program-missing', 'rm-no-such-program');
    expect(t(id, args)).toBe('This machine cannot find rm-no-such-program in its PATH. Install it there, or start Agora on that machine from a terminal where rm-no-such-program runs.');
    expect(refusalWords('start-program-missing', undefined)).toEqual(['start-program-missing']);
    expect(refusalWords('start-failed', 'x')).toEqual(['start-failed']);
  });
});
