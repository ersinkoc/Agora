// The desktop app's notifications, pure: the rules, the rate, the name made plain and the typed bridge (net/desktopNotify.ts).
import { describe, expect, it, vi } from 'vitest';
import { AGENT_ID_MAX, DESKTOP_NOTIFY_DEFAULTS, NOTIFY_GAP_MS, appNotify, flush, gate, inDesktopApp, notifiable, parseDesktopNotifyPrefs, sendToApp, shellEvents } from './desktopNotify';
import { openGate, type SoundEvent } from './sounds';

const away = { visible: false, focused: false };
const on = { ...DESKTOP_NOTIFY_DEFAULTS, on: true };
const fin = (agent: string): SoundEvent => ({ kind: 'finished', agent });
const needs = (agent: string): SoundEvent => ({ kind: 'needs-you', agent });

describe('the choice', () => {
  it('is off by default, and anything stored that is not understood is the default', () => {
    expect(DESKTOP_NOTIFY_DEFAULTS.on).toBe(false);
    expect(parseDesktopNotifyPrefs(undefined)).toEqual(DESKTOP_NOTIFY_DEFAULTS);
    expect(parseDesktopNotifyPrefs('{nope')).toEqual(DESKTOP_NOTIFY_DEFAULTS);
    expect(parseDesktopNotifyPrefs('[true]')).toEqual(DESKTOP_NOTIFY_DEFAULTS);
    expect(parseDesktopNotifyPrefs('{"on":"yes","finished":false}')).toEqual({ on: false, finished: false, needsYou: true });
  });
});

describe('the rules', () => {
  it('ask only when on, for a kind that is on, outside safe mode', () => {
    expect(notifiable(fin('a'), DESKTOP_NOTIFY_DEFAULTS, away, false)).toBe(false);
    expect(notifiable(fin('a'), on, away, false)).toBe(true);
    expect(notifiable(needs('a'), on, away, false)).toBe(true);
    expect(notifiable(fin('a'), on, away, true)).toBe(false);
    expect(notifiable(needs('a'), on, away, true)).toBe(false);
    expect(notifiable(fin('a'), { ...on, finished: false }, away, false)).toBe(false);
    expect(notifiable(needs('a'), { ...on, needsYou: false }, away, false)).toBe(false);
  });

  it('ask only while the window is hidden or not focused, whichever agent it is', () => {
    expect(notifiable(fin('a'), on, { visible: true, focused: true, viewing: 'b' }, false)).toBe(false);
    expect(notifiable(fin('a'), on, { visible: true, focused: false, viewing: 'a' }, false)).toBe(true);
    expect(notifiable(fin('a'), on, { visible: false, focused: true }, false)).toBe(true);
  });
});

describe('the rate', () => {
  it('sends a burst as one request, and at most one request in the gap; nothing is dropped', () => {
    const t0 = 1_000_000;
    const first = gate(openGate(), [fin('a'), needs('b')], t0);
    expect(first.send).toEqual([fin('a'), needs('b')]);
    const second = gate(first.next, [fin('c')], t0 + 500);
    expect(second.send).toBeUndefined();
    expect(second.waitMs).toBe(NOTIFY_GAP_MS - 500);
    const third = gate(second.next, [needs('d')], t0 + 900);
    expect(third.send).toBeUndefined();
    const later = flush(third.next, t0 + NOTIFY_GAP_MS, () => true);
    expect(later.send).toEqual([fin('c'), needs('d')]);
    expect(later.next.lastMs).toBe(t0 + NOTIFY_GAP_MS);
  });

  it('checks what waited again when the gap ends', () => {
    const t0 = 5_000;
    const a = gate(openGate(), [fin('a')], t0);
    const b = gate(a.next, [fin('b'), needs('c')], t0 + 10);
    const done = flush(b.next, t0 + NOTIFY_GAP_MS, (ev) => ev.agent !== 'b');
    expect(done.send).toEqual([needs('c')]);
    expect(flush(b.next, t0 + NOTIFY_GAP_MS, () => false).send).toBeUndefined();
  });

  it('keeps what waits bounded per kind, so many finished never push out a needs you', () => {
    const t0 = 0;
    const a = gate(openGate(), [fin('x')], t0);
    const many = Array.from({ length: 500 }, (_, i) => fin(`f${i}`));
    const b = gate(a.next, [needs('n'), ...many], t0 + 1);
    expect(b.next.pending.length).toBeLessThan(100);
    expect(b.next.pending).toContainEqual(needs('n'));
  });
});

describe('what is sent', () => {
  it('names an agent only by a plain name, and leaves out an event for no agent or an id too long', () => {
    const names: Record<string, string> = { a: 'codex · teal', b: 'evil‮txt', c: 'x'.repeat(25) };
    const out = shellEvents([fin('a'), needs('b'), fin('c'), { kind: 'finished' }, fin(''), fin('z'.repeat(AGENT_ID_MAX + 1)), fin('d')], (id) => names[id]);
    expect(out).toEqual([
      { kind: 'finished', agent: 'a', name: 'codex · teal' },
      { kind: 'needs-you', agent: 'b' },
      { kind: 'finished', agent: 'c' },
      { kind: 'finished', agent: 'd' },
    ]);
  });
});

describe('the bridge', () => {
  const shell = (invoke: (...a: unknown[]) => unknown) => ({ isTauri: true, __TAURI_INTERNALS__: { invoke } });

  it('is there only in the desktop app', () => {
    expect(inDesktopApp(shell(() => Promise.resolve()))).toBe(true);
    expect(inDesktopApp({})).toBe(false);
    expect(inDesktopApp(undefined)).toBe(false);
    // Both marks the app's shell sets, not one a page could have by chance.
    expect(inDesktopApp({ __TAURI_INTERNALS__: { invoke: () => 0 } })).toBe(false);
    expect(inDesktopApp({ isTauri: true, __TAURI_INTERNALS__: {} })).toBe(false);
  });

  it('calls the one command with the events, and ignores a refusal', async () => {
    const invoke = vi.fn(() => Promise.reject(new Error('not allowed')));
    const evs = [{ kind: 'finished' as const, agent: 'n/a', name: 'a' }];
    expect(sendToApp(evs, shell(invoke))).toBe(true);
    expect(invoke).toHaveBeenCalledWith('notify_agents', { events: evs });
    await Promise.resolve();
    expect(sendToApp([], shell(invoke))).toBe(false);
    expect(sendToApp(evs, {})).toBe(false);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(sendToApp(evs, shell(() => { throw new Error('gone'); }))).toBe(false);
  });

  it('is asked once whether the app has the command, and only an app that takes it counts', async () => {
    appNotify.reset();
    const yes = vi.fn((..._: unknown[]) => Promise.resolve());
    appNotify.probe(shell(yes));
    appNotify.probe(shell(yes));
    expect(yes).toHaveBeenCalledTimes(1);
    expect(yes).toHaveBeenCalledWith('notify_agents', { events: [] });
    expect(appNotify.supported()).toBe(false);
    await Promise.resolve(); await Promise.resolve();
    expect(appNotify.supported()).toBe(true);

    appNotify.reset();
    appNotify.probe(shell(() => Promise.reject(new Error('not allowed'))));
    await Promise.resolve(); await Promise.resolve();
    expect(appNotify.supported()).toBe(false);

    appNotify.reset();
    appNotify.probe({});
    expect(appNotify.supported()).toBe(false);
    appNotify.reset();
  });
});
