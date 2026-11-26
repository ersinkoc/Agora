// What changed for the person between two looks at the fleet (pure), and the "finished" mark that lasts until they see the agent
// (docs/ux/main-window.md, "Finished, not seen yet").
//
// An agent *finishes* when it goes from working to idle or finished: a turn ended, or its program exited by itself with status 0. One that
// ended with an error is Failed, and one stopped on request or interrupted with its machine (also drawn as finished once closed, net/view.ts)
// did not finish anything: neither has a sound or a mark. An agent *needs you* when it comes onto the Needs you list: blocked in its
// terminal or with a request waiting, it is one thing however many requests it opens, until it leaves the list. A machine asking to join
// is on that list too but has no sound: it is not an agent, and its card can arrive after the window's first look, which would sound for a
// request that was already there. An agent the window sees for the first time (the first snapshot, a sign-in again) is taken as it is: no
// sound, no mark.
import type { AgentTileModel, UiState } from '../model';
import type { AttentionItem } from './fleet';
import type { SoundEvent } from './sounds';

export interface Seen { states: ReadonlyMap<string, UiState>; asking: ReadonlySet<string> }

// Who an item on the Needs you list is about: its agent (a request names it even before the list has the agent), or the machine asking.
export const askerOf = (it: AttentionItem): string => (it.kind === 'approval' ? it.approval.agentId : it.kind === 'agent' ? it.agent.id : it.key);

// A closed agent finished only when its program exited by itself with status 0 (model.ts has what the exit codes mean).
const endedCleanly = (a: AgentTileModel): boolean => !a.closed || (a.exitCode === 0 && !a.interrupted);

export interface Changes {
  // Sounds that may play, at most one per agent and change.
  events: SoundEvent[];
  // Agents that finished now.
  finished: string[];
  // Agents at work again or waiting on the person: a mark they had is out of date.
  resumed: string[];
  // Agents that left the list.
  gone: string[];
  next: Seen;
}

export function changes(prev: Seen | undefined, agents: readonly AgentTileModel[], attention: readonly AttentionItem[]): Changes {
  const states = new Map<string, UiState>();
  for (const a of agents) states.set(a.id, a.state);
  const asking = new Set<string>();
  for (const it of attention) if (it.kind !== 'pair') asking.add(askerOf(it));
  const next: Seen = { states, asking };
  if (!prev) return { events: [], finished: [], resumed: [], gone: [], next };
  const events: SoundEvent[] = [];
  const finished: string[] = [];
  const resumed: string[] = [];
  for (const a of agents) {
    const was = prev.states.get(a.id);
    if (was === undefined || was === a.state) continue;
    if (a.state === 'working' || a.state === 'needs-you') resumed.push(a.id);
    else if (was === 'working' && (a.state === 'idle' || a.state === 'finished') && endedCleanly(a)) {
      finished.push(a.id);
      events.push({ kind: 'finished', agent: a.id });
    }
  }
  const gone = [...prev.states.keys()].filter((id) => !states.has(id));
  for (const who of asking) {
    if (prev.asking.has(who)) continue;
    events.push({ kind: 'needs-you', agent: who });
  }
  return { events, finished, resumed, gone, next };
}

// ---- The mark ---------------------------------------------------------------------------------------------------------------------------

// The mark's rules: an agent that finished while the person was not looking at it is marked; looking at it, its working again (or waiting
// on the person), or its leaving the list takes the mark away.
export function nextUnseen(cur: ReadonlySet<string>, c: { finished?: readonly string[]; resumed?: readonly string[]; gone?: readonly string[]; seen?: ReadonlySet<string> }): ReadonlySet<string> {
  const out = new Set(cur);
  for (const id of c.finished ?? []) if (!c.seen?.has(id)) out.add(id);
  for (const id of [...(c.resumed ?? []), ...(c.gone ?? [])]) out.delete(id);
  for (const id of out) if (c.seen?.has(id)) out.delete(id);
  return out.size === cur.size && [...out].every((id) => cur.has(id)) ? cur : out;
}

// The window's marks, in memory only: a reload is a new look at everything.
let unseen: ReadonlySet<string> = new Set();
const listeners = new Set<() => void>();
export const finishedMarks = {
  has: (id: string): boolean => unseen.has(id),
  all: (): ReadonlySet<string> => unseen,
  apply(c: Parameters<typeof nextUnseen>[1]) {
    const next = nextUnseen(unseen, c);
    if (next === unseen) return;
    unseen = next;
    listeners.forEach((l) => l());
  },
  subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
  // For tests and the gallery: set the marks as they are.
  set(ids: Iterable<string>) { unseen = new Set(ids); listeners.forEach((l) => l()); },
};
