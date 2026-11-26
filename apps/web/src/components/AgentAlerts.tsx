import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { AgentTileModel } from '../model';
import type { AttentionItem } from '../net/fleet';
import { changes, finishedMarks, type Seen } from '../net/finished';
import { onScreen } from '../net/onScreen';
import { audible, flush, gate, inFront, openGate, soundPrefs, type Gate, type SoundEvent, type SoundKind, type SoundPresence, type SoundVolume } from '../net/sounds';
import { SAFE } from '../plugins/safe';
import { playSound, unlockSounds } from './chime';

// Whether the window is shown and has the focus, read when it matters and again on every change of either.
export interface PagePresence { visible: boolean; focused: boolean }
const readPage = (): PagePresence => ({ visible: document.visibilityState === 'visible', focused: document.hasFocus() });
function usePagePresence(read: () => PagePresence): PagePresence {
  const [, bump] = useState(0);
  useEffect(() => {
    const on = () => bump((n) => n + 1);
    window.addEventListener('focus', on);
    window.addEventListener('blur', on);
    document.addEventListener('visibilitychange', on);
    return () => { window.removeEventListener('focus', on); window.removeEventListener('blur', on); document.removeEventListener('visibilitychange', on); };
  }, []);
  return read();
}

// What changed in the fleet, heard by the person: a sound when an agent finishes or needs you (net/sounds.ts has the rules), and the
// "finished" mark on an agent that finished while they were not looking at it (net/finished.ts). Draws nothing: the marks are drawn by the
// tiles, rows and tabs, and every sound has its sign on screen too (the state, the Needs you list, the mark), so a sound is never the only
// one. `ready`: the window has the fleet as it is (a hub window before its first snapshot, or after it signed out, has not), so the first
// look after it is taken as it is, with no sound and no mark.
export function AgentAlerts({ agents, attention, ready, safe = SAFE, page = readPage, play = playSound }: {
  agents: readonly AgentTileModel[];
  attention: readonly AttentionItem[];
  ready: boolean;
  safe?: boolean;
  page?: () => PagePresence;
  play?: (kind: SoundKind, volume: SoundVolume) => void;
}) {
  const screen = useSyncExternalStore(onScreen.subscribe, onScreen.get, onScreen.get);
  const presence = usePagePresence(page);
  const front = presence.visible && presence.focused;

  // The latest of everything a timer reads, so a sound that waited is checked against the window as it is when it plays.
  const seen = useRef<Seen | undefined>(undefined);
  const latest = useRef({ page, safe, play });
  latest.current = { page, safe, play };
  const where = (): SoundPresence => ({ ...latest.current.page(), viewing: onScreen.get().focused });
  const stillAudible = (ev: SoundEvent): boolean => {
    if (!audible(ev, soundPrefs.get(), soundPrefs.muted(), where(), latest.current.safe)) return false;
    // A sound that waited says something still true: the agent still asks, or is still finished.
    const s = seen.current;
    if (!s || ev.agent === undefined) return true;
    return ev.kind === 'needs-you' ? s.asking.has(ev.agent) : s.states.get(ev.agent) === 'idle' || s.states.get(ev.agent) === 'finished';
  };

  const gateRef = useRef<Gate>(openGate());
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const run = (r: { play?: SoundEvent; waitMs?: number; next: Gate }) => {
    gateRef.current = r.next;
    if (r.play) latest.current.play(r.play.kind, soundPrefs.get().volume);
    if (r.waitMs !== undefined && timer.current === undefined) {
      timer.current = setTimeout(() => { timer.current = undefined; run(flush(gateRef.current, Date.now(), stillAudible)); }, r.waitMs);
    }
  };
  useEffect(() => () => { if (timer.current !== undefined) clearTimeout(timer.current); }, []);

  // The audio may start only on an act of the person's: their first click or key in this visit starts it, when sounds are on.
  useEffect(() => {
    if (safe) return;
    const on = () => { if (soundPrefs.get().on) unlockSounds(); };
    window.addEventListener('pointerdown', on, true);
    window.addEventListener('keydown', on, true);
    return () => { window.removeEventListener('pointerdown', on, true); window.removeEventListener('keydown', on, true); };
  }, [safe]);

  useEffect(() => {
    // Not knowing the fleet (signed out, another hub): what was marked belongs to the last look, and the next look starts afresh.
    if (!ready) { if (seen.current) finishedMarks.set([]); seen.current = undefined; return; }
    const c = changes(seen.current, agents, attention);
    seen.current = c.next;
    const p = where();
    finishedMarks.apply({ finished: c.finished, resumed: c.resumed, gone: c.gone, seen: inFront(p) ? onScreen.get().visible : undefined });
    const evs = c.events.filter((ev) => audible(ev, soundPrefs.get(), soundPrefs.muted(), p, latest.current.safe));
    if (evs.length > 0) run(gate(gateRef.current, evs, Date.now()));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agents, attention, ready]);

  // Looking at an agent takes its mark away: its view on screen while the window is in front.
  useEffect(() => { if (front) finishedMarks.apply({ seen: screen.visible }); }, [front, screen]);
  return null;
}
