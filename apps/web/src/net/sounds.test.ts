import { afterEach, describe, expect, it, vi } from 'vitest';
import { MUTED_MAX, SOUND_DEFAULTS, SOUND_GAP_MS, audible, flush, gate, openGate, parseMuted, parseSoundPrefs, soundPrefs, type Gate, type SoundEvent, type SoundPrefs, type SoundPresence } from './sounds';

const on: SoundPrefs = { ...SOUND_DEFAULTS, on: true };
const away: SoundPresence = { visible: false, focused: false };
const front = (viewing?: string): SoundPresence => ({ visible: true, focused: true, viewing });
const fin = (agent?: string): SoundEvent => ({ kind: 'finished', agent });
const asks = (agent?: string): SoundEvent => ({ kind: 'needs-you', agent });

class Mem {
  m = new Map<string, string>();
  getItem(k: string) { return this.m.get(k) ?? null; }
  setItem(k: string, v: string) { this.m.set(k, v); }
}

afterEach(() => { soundPrefs.reset(); vi.useRealTimers(); });

describe('sound choices', () => {
  it('are off by default, so a window never surprises anyone', () => {
    expect(parseSoundPrefs(undefined)).toEqual(SOUND_DEFAULTS);
    expect(SOUND_DEFAULTS.on).toBe(false);
    expect(soundPrefs.get(new Mem())).toEqual(SOUND_DEFAULTS);
  });

  it('read anything stored as the defaults where it is not what they hold', () => {
    expect(parseSoundPrefs('not json')).toEqual(SOUND_DEFAULTS);
    expect(parseSoundPrefs('[1,2]')).toEqual(SOUND_DEFAULTS);
    expect(parseSoundPrefs(JSON.stringify({ on: 'yes', volume: 'max', finished: 0, needsYou: false, inFront: true }))).toEqual({ ...SOUND_DEFAULTS, needsYou: false, inFront: true });
  });

  it('round-trip through this browser\'s storage, and a new visit reads them back', () => {
    const kv = new Mem();
    soundPrefs.set({ on: true, volume: 'low', finished: false }, kv);
    soundPrefs.reset();
    expect(soundPrefs.get(kv)).toEqual({ ...SOUND_DEFAULTS, on: true, volume: 'low', finished: false });
    soundPrefs.set({ inFront: true }, kv);
    soundPrefs.reset();
    expect(soundPrefs.get(kv)).toEqual({ ...SOUND_DEFAULTS, on: true, volume: 'low', finished: false, inFront: true });
  });

  it('remember a muted agent across visits, and unmuting forgets it', () => {
    const kv = new Mem();
    soundPrefs.setMuted('m1/a1', true, kv);
    soundPrefs.setMuted('m1/a2', true, kv);
    soundPrefs.reset();
    expect(soundPrefs.muted(kv)).toEqual(['m1/a1', 'm1/a2']);
    soundPrefs.setMuted('m1/a1', false, kv);
    soundPrefs.reset();
    expect(soundPrefs.isMuted('m1/a1', kv)).toBe(false);
    expect(soundPrefs.isMuted('m1/a2', kv)).toBe(true);
  });

  it('keep at most MUTED_MAX muted agents, dropping the oldest, and ignore what is not a key', () => {
    const many = Array.from({ length: MUTED_MAX + 5 }, (_, i) => `m/${i}`);
    expect(parseMuted(JSON.stringify([...many, 3, '', null]))).toEqual(many.slice(5));
    expect(parseMuted('{}')).toEqual([]);
  });

  it('hold for the visit when storage refuses', () => {
    const kv = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('full'); } };
    soundPrefs.set({ on: true }, kv);
    expect(soundPrefs.get(kv).on).toBe(true);
  });

  it('tell whoever listens when they change', () => {
    const heard = vi.fn();
    const off = soundPrefs.subscribe(heard);
    soundPrefs.set({ on: true }, new Mem());
    soundPrefs.setMuted('x', true, new Mem());
    off();
    soundPrefs.set({ on: false }, new Mem());
    expect(heard).toHaveBeenCalledTimes(2);
  });
});

describe('whether a sound may play', () => {
  it('never in safe mode, whatever is chosen', () => {
    for (const p of [away, front()]) for (const ev of [fin('a'), asks('a'), asks()]) expect(audible(ev, { ...on, inFront: true }, [], p, true)).toBe(false);
  });

  it('never while sounds are off, that sound is off, or the agent is muted', () => {
    expect(audible(fin('a'), SOUND_DEFAULTS, [], away, false)).toBe(false);
    expect(audible(fin('a'), { ...on, finished: false }, [], away, false)).toBe(false);
    expect(audible(asks('a'), { ...on, finished: false }, [], away, false)).toBe(true);
    expect(audible(asks('a'), { ...on, needsYou: false }, [], away, false)).toBe(false);
    expect(audible(asks('a'), on, ['a'], away, false)).toBe(false);
    expect(audible(asks('b'), on, ['a'], away, false)).toBe(true);
    // An event for no agent is not silenced by an agent's mute.
    expect(audible(asks(), on, ['a'], away, false)).toBe(true);
  });

  it('only while the window is hidden or not focused, unless asked for in front too', () => {
    expect(audible(fin('a'), on, [], away, false)).toBe(true);
    expect(audible(fin('a'), on, [], { visible: true, focused: false }, false)).toBe(true);
    expect(audible(fin('a'), on, [], { visible: false, focused: true }, false)).toBe(true);
    expect(audible(fin('a'), on, [], front(), false)).toBe(false);
    expect(audible(fin('a'), { ...on, inFront: true }, [], front(), false)).toBe(true);
  });

  it('never for the agent in the focused pane while the window is in front, even when asked for in front', () => {
    const p = { ...on, inFront: true };
    expect(audible(fin('a'), p, [], front('a'), false)).toBe(false);
    expect(audible(asks('a'), p, [], front('a'), false)).toBe(false);
    expect(audible(fin('b'), p, [], front('a'), false)).toBe(true);
    // In the background the person is not looking at the focused pane: it plays.
    expect(audible(fin('a'), on, [], { visible: false, focused: false, viewing: 'a' }, false)).toBe(true);
  });
});

describe('a burst is one sound', () => {
  it('plays the first at once and holds the rest of the second', () => {
    let g: Gate = openGate();
    let r = gate(g, [fin('a')], 10_000);
    expect(r.play).toEqual(fin('a'));
    g = r.next;
    r = gate(g, [fin('b')], 10_300);
    expect(r.play).toBeUndefined();
    expect(r.waitMs).toBe(SOUND_GAP_MS - 300);
    g = r.next;
    r = gate(g, [fin('c')], 10_600);
    expect(r.play).toBeUndefined();
    expect(r.waitMs).toBe(SOUND_GAP_MS - 600);
    // At the end of the gap: one sound for the two that waited.
    r = flush(r.next, 11_000, () => true);
    expect(r.play?.kind).toBe('finished');
    expect(r.next.pending).toEqual([]);
    expect(r.next.lastMs).toBe(11_000);
  });

  it('plays the most urgent of a burst: needs you before finished', () => {
    expect(gate(openGate(), [fin('a'), asks('b'), fin('c')], 0).play).toEqual(asks('b'));
    const held = gate(gate(openGate(), [fin('a')], 0).next, [fin('b'), asks('c')], 500);
    expect(flush(held.next, 1000, () => true).play).toEqual(asks('c'));
  });

  it('checks what waited again at the end of the gap', () => {
    const held = gate(gate(openGate(), [fin('a')], 0).next, [asks('b')], 200);
    const r = flush(held.next, 1000, (ev) => ev.agent !== 'b');
    expect(r.play).toBeUndefined();
    expect(r.waitMs).toBeUndefined();
    expect(r.next.lastMs).toBe(0);
  });

  it('never plays twice within a second, by the clock rather than by a timer', () => {
    vi.useFakeTimers();
    vi.setSystemTime(50_000);
    const played: number[] = [];
    let g: Gate = openGate();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const step = (r: ReturnType<typeof gate>) => {
      g = r.next;
      if (r.play) played.push(Date.now());
      if (r.waitMs !== undefined && !timer) timer = setTimeout(() => { timer = undefined; step(flush(g, Date.now(), () => true)); }, r.waitMs);
    };
    for (let i = 0; i < 25; i++) { step(gate(g, [fin(`a${i}`)], Date.now())); vi.advanceTimersByTime(130); }
    vi.advanceTimersByTime(2000);
    for (let i = 1; i < played.length; i++) expect((played[i] as number) - (played[i - 1] as number)).toBeGreaterThanOrEqual(SOUND_GAP_MS);
    // 25 events from 50.00 s to 53.12 s: one sound a second (at 50, 51, 52, 53 and 54 s), not 25.
    expect(played.map((t) => t - 50_000)).toEqual([0, 1000, 2000, 3000, 4000]);
  });
});
