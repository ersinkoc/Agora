// @vitest-environment jsdom
// ADR-0068 rung 4: the whole window, connected to a stand-in hub as its owner. Every command the window has on screen is in the command
// palette, under its name, with the key it has now; a key changed in Settings > Keyboard is the key the palette, the title bar and a key
// press use; nothing a key or the palette does can answer a request.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { App } from '../App';
import { forgetLoaded } from '../layout/store';
import { attentionPrefs } from '../net/attention';
import { soundPrefs } from '../net/sounds';
import { CONTROL_KIND, encodeFrame, EVENT_KIND, STREAM_CONTROL, STREAM_EVENTS } from '../net/frame';
import { setLanguage, t } from '../i18n';
import { COMMANDS, commandById } from './commands';
import { withKeys } from './file';
import { canRun, handlerOf, registeredCommands } from './registry';
import { keyStore } from './store';
import { shownKey } from './view';
import { commandLabel } from './view';
// The keys are shown as on Linux and Windows whatever system runs the test (a Mac shows its own symbols).
vi.mock('./strokes', async (original) => ({ ...(await original<typeof import('./strokes')>()), IS_MAC: false }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const v1 = agora.v1;

class HubSocket {
  static last: HubSocket | undefined;
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(readonly url: string, readonly protocols: string[]) { HubSocket.last = this; }
  send() {}
  close() {}
  deliver(streamId: number, kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
}

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  attentionPrefs.reset();
  forgetLoaded();
  keyStore.reset();
  window.localStorage.setItem('agora.hubToken', 'tok');
  vi.stubGlobal('WebSocket', HubSocket);
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })));
  vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('min-width'), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }));
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  keyStore.reset();
  window.localStorage.clear();
  window.location.hash = '';
});

const tick = async (n: number) => { for (let i = 0; i < n; i++) await act(async () => { await new Promise((r) => setTimeout(r, 50)); }); };
async function window_() {
  window.location.hash = '#/';
  await act(async () => { root.render(<App />); });
  const s = HubSocket.last as HubSocket;
  await act(async () => {
    s.onopen?.();
    s.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, v1.Welcome.encode({ chosenGeneration: 1, personRole: v1.PersonRole.PERSON_ROLE_OWNER }).finish());
    s.deliver(STREAM_EVENTS, EVENT_KIND.SNAPSHOT, v1.Snapshot.encode({
      pos: 3, nodes: [{ nodeId: 'n1', hostname: 'Box', online: true }],
      agents: [{ agentId: 'n1/a1', nodeId: 'n1', kind: 'codex', running: true }, { agentId: 'n1/a2', nodeId: 'n1', kind: 'claude', running: true }],
      terminals: [{ terminalId: 't-a1', nodeId: 'n1', agentId: 'n1/a1' }],
      approvals: [{ approvalId: 'n1/r1', agentId: 'n1/a1', action: 'Edit', detail: 'src/auth.rs' }],
    }).finish());
  });
  await tick(3);
}
const press = (init: KeyboardEventInit, target: EventTarget = document.body) => act(() => { target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })); });
const palette = () => document.querySelector('.palette');
const option = (id: string) => document.getElementById(`palette-${id}`);

// Commands the window offers only in some states, and why they are not on screen in this one.
const ELSEWHERE: Record<string, string> = {
  'window.lock': 'only with a screen password set',
  'machines.allow': 'only while this hub can open the window for new machines',
};

describe('the command palette lists every command', () => {
  it('every command has a handler in the window (or a stated reason why not here), and is listed with its key', async () => {
    await window_();
    const registered = new Set(registeredCommands());
    for (const c of COMMANDS) expect(registered.has(c.id) || c.id in ELSEWHERE, `${c.id} has no handler on screen`).toBe(true);
    press({ key: 'k', code: 'KeyK', ctrlKey: true });
    expect(palette()).not.toBeNull();
    let listed = 0;
    for (const c of COMMANDS) {
      const row = option(c.id);
      if (c.notInPalette || !canRun(c.id)) { expect(row, c.id).toBeNull(); continue; }
      listed++;
      expect(row, c.id).not.toBeNull();
      expect(row?.querySelector('.palette__label')?.textContent).toBe(handlerOf(c.id)?.label ?? commandLabel(c));
      const key = shownKey(keyStore.state().map, c.id);
      expect(row?.querySelector('kbd')?.textContent ?? undefined, c.id).toBe(key);
    }
    expect(listed).toBeGreaterThan(25);
    // The defaults as before: N beside "Next that needs me", Alt+1 beside the first tab, the side panel's Alt+Shift+1.
    expect(option('attention.next')?.querySelector('kbd')?.textContent).toBe('N');
    expect(option('attention.next')?.getAttribute('aria-keyshortcuts')).toBe('N');
    expect(option('window.agents')?.getAttribute('aria-keyshortcuts')).toBe('Control+B');
    expect(option('side.place-1')?.querySelector('.palette__label')?.textContent).toBe('Side panel: Needs you');
    expect(option('window.safe-mode')?.querySelector('kbd')?.textContent).toBe('Ctrl+Alt+Shift+S');
  });

  it('turns sounds on, and mutes the focused agent\'s sounds, by the palette', async () => {
    vi.stubGlobal('AudioContext', class { state = 'running'; resume() { return Promise.resolve(); } });
    soundPrefs.reset();
    await window_();
    press({ key: 'k', code: 'KeyK', ctrlKey: true });
    expect(option('sounds.toggle')?.querySelector('.palette__label')?.textContent).toBe('Turn sounds on');
    // Sounds off: muting an agent is not offered.
    expect(option('agent.mute-sounds')).toBeNull();
    act(() => option('sounds.toggle')?.click());
    expect(soundPrefs.get().on).toBe(true);
    // An agent's tab in focus (Alt+2: the first agent after Home), then the palette again.
    press({ key: '2', code: 'Digit2', altKey: true });
    press({ key: 'k', code: 'KeyK', ctrlKey: true });
    expect(option('sounds.toggle')?.querySelector('.palette__label')?.textContent).toBe('Turn sounds off');
    const mute = option('agent.mute-sounds');
    expect(mute?.querySelector('.palette__label')?.textContent).toMatch(/^Mute the sounds of /);
    act(() => mute?.click());
    expect(soundPrefs.muted().length).toBe(1);
    soundPrefs.reset();
  });

  it('is searchable by a command\'s name', async () => {
    await window_();
    press({ key: 'k', code: 'KeyK', ctrlKey: true });
    const input = document.querySelector<HTMLInputElement>('.palette input') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    act(() => { setter?.call(input, 'four (2'); input.dispatchEvent(new Event('input', { bubbles: true })); });
    const first = document.querySelector('[role="option"][aria-selected="true"]');
    expect(first?.id).toBe('palette-layout.preset-2x2');
  });

  it('a key changed in Settings > Keyboard is the key everywhere: the palette, the title bar, a key press', async () => {
    keyStore.set(withKeys(keyStore.state().file, 'palette.open', ['ctrl+p']));
    await window_();
    expect(host.querySelector('header kbd')?.textContent).toBe('Ctrl P');
    press({ key: 'k', code: 'KeyK', ctrlKey: true });
    expect(palette()).toBeNull();
    press({ key: 'p', code: 'KeyP', ctrlKey: true });
    expect(palette()).not.toBeNull();
    expect(option('go.home')?.querySelector('kbd')).toBeNull();
  });

  it('with nothing changed the title bar says Ctrl K, as before', async () => {
    await window_();
    expect(host.querySelector('header kbd')?.textContent).toBe('Ctrl K');
  });

  it('no command answers a request, and the palette offers no Allow or Deny', async () => {
    await window_();
    press({ key: 'k', code: 'KeyK', ctrlKey: true });
    const labels = [...document.querySelectorAll('.palette__label')].map((l) => l.textContent ?? '');
    for (const word of [t('approval-allow'), t('approval-deny')]) expect(labels.some((l) => l === word)).toBe(false);
    for (const id of registeredCommands()) expect(commandById(id), id).toBeDefined();
  });
});
