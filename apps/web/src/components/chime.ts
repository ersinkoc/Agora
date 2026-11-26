// The two sounds, made in the browser (WebAudio oscillators with a short envelope): no audio file, nothing to license, nothing to fetch.
// "Finished" is two soft rising notes; "needs you" is two brighter notes at one pitch, a little firmer. Each lasts under half a second.
//
// A browser lets a page play sound only after the person has interacted with it: the audio is started on their first click or key in this
// visit (and on the Sounds switch in Settings), and until then nothing plays. Never in safe mode (ADR-0114): no audio is made there at all.
import { SAFE } from '../plugins/safe';
import type { SoundKind, SoundVolume } from '../net/sounds';

// The peak level of each step, kept low: these are cues, not alarms.
export const VOLUME_GAIN: Record<SoundVolume, number> = { low: 0.04, medium: 0.1, high: 0.2 };

// The notes: when each starts (s, after the first), its pitch (Hz), how long it rings (s) and its wave.
interface Note { at: number; hz: number; dur: number; wave: OscillatorType }
export const NOTES: Record<SoundKind, readonly Note[]> = {
  finished: [{ at: 0, hz: 659.25, dur: 0.18, wave: 'sine' }, { at: 0.11, hz: 987.77, dur: 0.3, wave: 'sine' }],
  'needs-you': [{ at: 0, hz: 880, dur: 0.14, wave: 'triangle' }, { at: 0.17, hz: 880, dur: 0.2, wave: 'triangle' }],
};

type Ctor = new () => AudioContext;
const ctorOf = (): Ctor | undefined => {
  if (typeof window === 'undefined') return undefined;
  const w = window as unknown as { AudioContext?: Ctor; webkitAudioContext?: Ctor };
  return w.AudioContext ?? w.webkitAudioContext;
};

/** Whether this browser can make the sounds at all. */
export const canPlaySounds = (): boolean => !SAFE && !!ctorOf();

let ctx: AudioContext | undefined;
function context(): AudioContext | undefined {
  if (SAFE) return undefined;
  if (ctx) return ctx;
  const C = ctorOf();
  if (!C) return undefined;
  try { ctx = new C(); } catch { return undefined; }
  return ctx;
}

/** Starts the audio on an act of the person's (a click, a key, the switch), so later sounds may play. */
export function unlockSounds(): void {
  const c = context();
  if (c && c.state === 'suspended') void c.resume().catch(() => { /* still not allowed; a later act tries again */ });
}

/** Plays one sound now. A browser that has not allowed audio yet gets one try to start it; a sound that could not start within a moment
 *  is dropped rather than played late. */
export function playSound(kind: SoundKind, volume: SoundVolume): void {
  const c = context();
  if (!c) return;
  const go = () => {
    try {
      const t0 = c.currentTime + 0.01;
      const peak = VOLUME_GAIN[volume];
      for (const n of NOTES[kind]) {
        const osc = c.createOscillator();
        const gain = c.createGain();
        osc.type = n.wave;
        osc.frequency.setValueAtTime(n.hz, t0 + n.at);
        gain.gain.setValueAtTime(0.0001, t0 + n.at);
        gain.gain.linearRampToValueAtTime(peak, t0 + n.at + 0.012);
        gain.gain.exponentialRampToValueAtTime(0.0001, t0 + n.at + n.dur);
        osc.connect(gain).connect(c.destination);
        osc.start(t0 + n.at);
        osc.stop(t0 + n.at + n.dur + 0.02);
      }
    } catch { /* the audio went away: no sound this time */ }
  };
  if (c.state !== 'suspended') { go(); return; }
  const asked = Date.now();
  void c.resume().then(() => { if (Date.now() - asked < LATE_MS) go(); }, () => { /* not allowed yet */ });
}
// A sound that could only start this much later is not played: it would no longer say what just happened.
const LATE_MS = 1000;
