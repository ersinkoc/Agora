// @vitest-environment jsdom
// The sounds and the finished mark in a DOM: the real AgentAlerts, the real tile and the real Settings card, with the audio replaced by a
// recorder. Each rule of net/sounds.ts and net/finished.ts that the window has to wire up (safe mode, the focused pane, the window in front,
// a burst, the mark set and cleared, the choices kept) has a test here that fails without its wiring.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel, UiState } from '../model';
import { attentionList } from '../net/fleet';
import { finishedMarks } from '../net/finished';
import { onScreen } from '../net/onScreen';
import { SOUND_GAP_MS, soundPrefs, type SoundKind } from '../net/sounds';
import { AgentAlerts, type PagePresence } from './AgentAlerts';
import { AgentTile } from './AgentTile';
import { CloseAgentMenu } from './CloseAgent';
import { SoundSettings } from './SoundSettings';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const agent = (id: string, state: UiState, more: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: 'var(--agent-amber)', machine: 'laptop', workspace: 'w', state, preview: '', ...more });

let host: HTMLElement;
let root: Root;
let page: PagePresence;
const played: SoundKind[] = [];
const play = (k: SoundKind) => { played.push(k); };
const screenOwner = Symbol('test');

// The audio: a stand-in that the switch can find (jsdom has none).
class FakeAudio { state = 'running'; resume() { return Promise.resolve(); } }

function draw(agents: AgentTileModel[], opts: { safe?: boolean; ready?: boolean } = {}) {
  act(() => root.render(<>
    <AgentAlerts agents={agents} attention={attentionList([], agents)} ready={opts.ready ?? true} safe={opts.safe ?? false} page={() => page} play={play} />
    {agents.map((a) => <AgentTile key={a.id} agent={a} />)}
  </>));
}

beforeEach(() => {
  vi.useFakeTimers();
  setLanguage('en');
  localStorage.clear();
  soundPrefs.reset();
  finishedMarks.set([]);
  played.length = 0;
  page = { visible: false, focused: false };
  (window as unknown as { AudioContext?: unknown }).AudioContext = FakeAudio;
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  onScreen.withdraw(screenOwner);
  delete (window as unknown as { AudioContext?: unknown }).AudioContext;
  vi.useRealTimers();
});

describe('sounds', () => {
  it('play nothing while sounds are off (the default)', () => {
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    expect(played).toEqual([]);
  });

  it('play "finished" when an agent ends its turn while the window is in the background', () => {
    soundPrefs.set({ on: true });
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    expect(played).toEqual(['finished']);
  });

  it('play nothing in safe mode, whatever is chosen', () => {
    soundPrefs.set({ on: true, inFront: true });
    draw([agent('a', 'working'), agent('b', 'working')], { safe: true });
    draw([agent('a', 'idle'), agent('b', 'needs-you')], { safe: true });
    act(() => { vi.advanceTimersByTime(5000); });
    expect(played).toEqual([]);
  });

  it('play nothing for the first look, nor while the fleet is not known yet', () => {
    soundPrefs.set({ on: true });
    draw([], { ready: false });
    draw([agent('a', 'idle'), agent('b', 'needs-you')]);
    expect(played).toEqual([]);
  });

  it('never for the agent in the focused pane while the window is in front, even when asked for in front', () => {
    soundPrefs.set({ on: true, inFront: true });
    page = { visible: true, focused: true };
    onScreen.publish(screenOwner, { focused: 'a', visible: new Set(['a']) });
    draw([agent('a', 'working'), agent('b', 'working')]);
    draw([agent('a', 'idle'), agent('b', 'working')]);
    expect(played).toEqual([]);
    draw([agent('a', 'idle'), agent('b', 'idle')]);
    expect(played).toEqual(['finished']);
  });

  it('play only in the background unless asked for in front too', () => {
    soundPrefs.set({ on: true });
    page = { visible: true, focused: true };
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    expect(played).toEqual([]);
  });

  it('make one sound of a burst, the most urgent, and at most one a second', () => {
    soundPrefs.set({ on: true });
    draw([agent('a', 'working'), agent('b', 'working'), agent('c', 'working')]);
    draw([agent('a', 'idle'), agent('b', 'working'), agent('c', 'working')]);
    expect(played).toEqual(['finished']);
    act(() => { vi.advanceTimersByTime(200); });
    draw([agent('a', 'idle'), agent('b', 'idle'), agent('c', 'needs-you')]);
    expect(played).toEqual(['finished']);
    act(() => { vi.advanceTimersByTime(SOUND_GAP_MS); });
    expect(played).toEqual(['finished', 'needs-you']);
  });

  it('leave out a muted agent', () => {
    soundPrefs.set({ on: true });
    soundPrefs.setMuted('a', true);
    draw([agent('a', 'working')]);
    draw([agent('a', 'needs-you')]);
    expect(played).toEqual([]);
  });
});

describe('the finished mark', () => {
  const marked = () => [...host.querySelectorAll('[data-finished-unseen]')].length;

  it('stays on the tile of an agent that finished while the person was away, with words for a screen reader', () => {
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    expect(marked()).toBe(1);
    const tile = host.querySelector('button.tile') as HTMLElement;
    expect(tile.getAttribute('aria-label')).toContain('Finished while you were away');
    // Coming back to the window is not looking at the agent.
    page = { visible: true, focused: true };
    act(() => { window.dispatchEvent(new Event('focus')); });
    expect(marked()).toBe(1);
  });

  it('is cleared when the person views the agent', () => {
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    page = { visible: true, focused: true };
    act(() => { onScreen.publish(screenOwner, { focused: 'a', visible: new Set(['a']) }); });
    expect(marked()).toBe(0);
  });

  it('is not set for an agent the person was looking at, and needs you is not finished', () => {
    page = { visible: true, focused: true };
    onScreen.publish(screenOwner, { visible: new Set(['a']) });
    draw([agent('a', 'working'), agent('b', 'working')]);
    draw([agent('a', 'idle'), agent('b', 'needs-you')]);
    expect(marked()).toBe(0);
  });

  it('goes when the agent works again', () => {
    draw([agent('a', 'working')]);
    draw([agent('a', 'idle')]);
    draw([agent('a', 'working')]);
    expect(marked()).toBe(0);
  });
});

describe('the choices', () => {
  const sw = (name: string) => [...host.querySelectorAll<HTMLButtonElement>('[role="switch"]')].find((b) => document.getElementById(b.getAttribute('aria-labelledby') ?? '')?.textContent === name) as HTMLButtonElement;

  it('turn sounds on in Settings, keep the volume and each sound, and read them back on a new visit', () => {
    act(() => root.render(<SoundSettings />));
    expect(sw('Play sounds').getAttribute('aria-checked')).toBe('false');
    expect(host.textContent).not.toContain('Volume');
    act(() => sw('Play sounds').click());
    act(() => (host.querySelector<HTMLButtonElement>('[role="radio"]:first-child') ?? [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Quiet'))?.click());
    act(() => sw('When an agent finishes').click());
    act(() => sw('Also while this window is in front').click());
    soundPrefs.reset();
    expect(soundPrefs.get()).toEqual({ on: true, volume: 'low', finished: false, needsYou: true, inFront: true });
  });

  it('cannot be turned on where the browser makes no sound', () => {
    delete (window as unknown as { AudioContext?: unknown }).AudioContext;
    act(() => root.render(<SoundSettings />));
    expect(sw('Play sounds').disabled).toBe(true);
    expect(host.textContent).toContain('This browser cannot play sounds.');
  });

  it('mute one agent from its menu while sounds are on, remembered in this browser', () => {
    const a = agent('m/a', 'working');
    act(() => root.render(<CloseAgentMenu agent={a} />));
    // Sounds off and nothing to close here: no menu at all.
    expect(host.querySelector('button')).toBeNull();
    act(() => soundPrefs.set({ on: true }));
    act(() => host.querySelector<HTMLButtonElement>('button')?.click());
    const item = [...document.querySelectorAll<HTMLElement>('[role="menuitemradio"], [role="menuitem"]')].find((x) => x.textContent?.includes("Mute this agent's sounds"));
    expect(item).toBeTruthy();
    act(() => item?.click());
    soundPrefs.reset();
    expect(soundPrefs.isMuted('m/a')).toBe(true);
  });
});
