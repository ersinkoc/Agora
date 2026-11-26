// @vitest-environment jsdom
// The desktop app's notifications in a DOM: the real AgentAlerts asking the app (a recorder in its place), the Settings card, and the
// browser's notification switch pointing to it inside the app. Each rule of net/desktopNotify.ts the window wires up has a test here.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel, UiState } from '../model';
import { NOTIFY_GAP_MS, appNotify, desktopNotifyPrefs, type ShellEvent } from '../net/desktopNotify';
import { attentionList } from '../net/fleet';
import { finishedMarks } from '../net/finished';
import { onScreen } from '../net/onScreen';
import { soundPrefs } from '../net/sounds';
import { AgentAlerts, type PagePresence } from './AgentAlerts';
import { AppNotifySettings } from './AppNotifySettings';
import { AttentionSettings } from './AttentionSettings';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const agent = (id: string, state: UiState, more: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: 'var(--agent-amber)', machine: 'laptop', workspace: 'w', state, preview: '', ...more });

let host: HTMLElement;
let root: Root;
let page: PagePresence;
const asked: ShellEvent[][] = [];
const notify = (evs: ShellEvent[]) => { asked.push(evs); };
const screenOwner = Symbol('test');

function draw(agents: AgentTileModel[], opts: { safe?: boolean; inApp?: boolean } = {}) {
  act(() => root.render(
    <AgentAlerts agents={agents} attention={attentionList([], agents)} ready safe={opts.safe ?? false} page={() => page} play={() => {}}
      inApp={opts.inApp ?? true} notify={notify} />,
  ));
}

beforeEach(() => {
  vi.useFakeTimers();
  setLanguage('en');
  localStorage.clear();
  soundPrefs.reset();
  desktopNotifyPrefs.reset();
  appNotify.reset();
  finishedMarks.set([]);
  asked.length = 0;
  page = { visible: false, focused: false };
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  onScreen.withdraw(screenOwner);
  vi.useRealTimers();
});

describe('the app is asked for a notification', () => {
  it('never while they are off (the default), nor outside the app', () => {
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    expect(asked).toEqual([]);
    desktopNotifyPrefs.set({ on: true });
    draw([agent('b', 'working')], { inApp: false });
    draw([agent('b', 'idle')], { inApp: false });
    expect(asked).toEqual([]);
  });

  it('when an agent finishes or needs you while the window is in the background, with its plain name', () => {
    desktopNotifyPrefs.set({ on: true });
    draw([agent('a', 'working'), agent('b', 'working', { name: 'x‮y' })]);
    draw([agent('a', 'idle'), agent('b', 'needs-you', { name: 'x‮y' })]);
    expect(asked).toEqual([[{ kind: 'finished', agent: 'a', name: 'claude · a' }, { kind: 'needs-you', agent: 'b' }]]);
  });

  it('never in safe mode', () => {
    desktopNotifyPrefs.set({ on: true });
    draw([agent('a', 'working')], { safe: true });
    draw([agent('a', 'idle')], { safe: true });
    act(() => { vi.advanceTimersByTime(5 * NOTIFY_GAP_MS); });
    expect(asked).toEqual([]);
  });

  it('never while the window is in front, whichever agent is focused', () => {
    desktopNotifyPrefs.set({ on: true });
    page = { visible: true, focused: true };
    onScreen.publish(screenOwner, { focused: 'b', visible: new Set(['b']) });
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    expect(asked).toEqual([]);
    // Not focused (another app in front): asked, even for the agent in the focused pane.
    page = { visible: true, focused: false };
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    expect(asked).toHaveLength(1);
  });

  it('for a kind that is on only', () => {
    desktopNotifyPrefs.set({ on: true, finished: false });
    draw([agent('a', 'working'), agent('b', 'working')]);
    draw([agent('a', 'idle'), agent('b', 'needs-you')]);
    expect(asked).toEqual([[{ kind: 'needs-you', agent: 'b', name: 'claude · b' }]]);
  });

  it('at most once in 2 s; what came meanwhile goes with the next, checked again', () => {
    desktopNotifyPrefs.set({ on: true });
    draw([agent('a', 'working'), agent('b', 'working'), agent('c', 'working')]);
    draw([agent('a', 'idle'), agent('b', 'working'), agent('c', 'working')]);
    expect(asked).toHaveLength(1);
    act(() => { vi.advanceTimersByTime(300); });
    draw([agent('a', 'idle'), agent('b', 'idle'), agent('c', 'needs-you')]);
    expect(asked).toHaveLength(1);
    act(() => { vi.advanceTimersByTime(NOTIFY_GAP_MS); });
    expect(asked[1]).toEqual([{ kind: 'finished', agent: 'b', name: 'claude · b' }, { kind: 'needs-you', agent: 'c', name: 'claude · c' }]);
    // A burst that the person came back to before the gap ended is not sent.
    act(() => { vi.advanceTimersByTime(100); });
    draw([agent('a', 'working'), agent('b', 'idle'), agent('c', 'needs-you')]);
    draw([agent('a', 'idle'), agent('b', 'idle'), agent('c', 'needs-you')]);
    page = { visible: true, focused: true };
    act(() => { vi.advanceTimersByTime(NOTIFY_GAP_MS); });
    expect(asked).toHaveLength(2);
  });

  it('through the app\'s own command by default, once the app said it has it', async () => {
    desktopNotifyPrefs.set({ on: true });
    const invoke = vi.fn((..._: unknown[]) => Promise.resolve());
    const w = window as unknown as { isTauri?: boolean; __TAURI_INTERNALS__?: unknown };
    w.isTauri = true;
    w.__TAURI_INTERNALS__ = { invoke };
    try {
      const go = (agents: AgentTileModel[]) => act(() => root.render(<AgentAlerts agents={agents} attention={attentionList([], agents)} ready safe={false} page={() => page} play={() => {}} />));
      go([agent('a', 'working')]);
      // The page asks the app once, with no event.
      expect(invoke).toHaveBeenCalledWith('notify_agents', { events: [] });
      await act(async () => { await Promise.resolve(); });
      go([agent('a', 'idle')]);
      expect(invoke).toHaveBeenLastCalledWith('notify_agents', { events: [{ kind: 'finished', agent: 'a', name: 'claude · a' }] });
      expect(invoke).toHaveBeenCalledTimes(2);
    } finally {
      delete w.isTauri;
      delete w.__TAURI_INTERNALS__;
    }
  });

  it('never from an app built before App notifications (it refuses the command), which keeps the browser\'s', async () => {
    desktopNotifyPrefs.set({ on: true });
    const invoke = vi.fn((..._: unknown[]) => Promise.reject(new Error('Command notify_agents not allowed by ACL')));
    const w = window as unknown as { isTauri?: boolean; __TAURI_INTERNALS__?: unknown };
    w.isTauri = true;
    w.__TAURI_INTERNALS__ = { invoke };
    try {
      const go = (agents: AgentTileModel[]) => act(() => root.render(<>
        <AgentAlerts agents={agents} attention={attentionList([], agents)} ready safe={false} page={() => page} play={() => {}} />
        <AttentionSettings />
      </>));
      go([agent('a', 'working')]);
      await act(async () => { await Promise.resolve(); });
      go([agent('a', 'idle')]);
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(appNotify.supported()).toBe(false);
      expect(host.textContent).not.toContain('In the app, use App notifications below.');
    } finally {
      delete w.isTauri;
      delete w.__TAURI_INTERNALS__;
    }
  });
});

describe('the choices', () => {
  it('turn the app\'s notifications on in Settings, each kind, kept for the next visit', () => {
    act(() => root.render(<AppNotifySettings safe={false} />));
    const main = host.querySelector<HTMLButtonElement>('[role="switch"]');
    expect(main?.getAttribute('aria-checked')).toBe('false');
    expect(host.textContent).toContain('Off until you turn them on.');
    act(() => { main?.click(); });
    expect(desktopNotifyPrefs.get().on).toBe(true);
    const switches = host.querySelectorAll<HTMLButtonElement>('[role="switch"]');
    expect(switches).toHaveLength(3);
    act(() => { switches[1]?.click(); });
    desktopNotifyPrefs.reset();
    expect(desktopNotifyPrefs.get()).toEqual({ on: true, finished: false, needsYou: true });
  });

  it('cannot be turned on in safe mode, and say why', () => {
    desktopNotifyPrefs.set({ on: true });
    act(() => root.render(<AppNotifySettings safe />));
    const main = host.querySelector<HTMLButtonElement>('[role="switch"]');
    expect(main?.getAttribute('aria-checked')).toBe('false');
    expect(main?.disabled).toBe(true);
    expect(host.textContent).toContain('No notifications in safe mode.');
  });

  it('point the browser\'s notification switch to the app\'s own inside the app', () => {
    act(() => root.render(<AttentionSettings inApp />));
    expect(host.textContent).toContain('In the app, use App notifications below.');
    const switches = [...host.querySelectorAll<HTMLButtonElement>('[role="switch"]')];
    expect(switches.some((s) => s.disabled)).toBe(true);
  });
});
