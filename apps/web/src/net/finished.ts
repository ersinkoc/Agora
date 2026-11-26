// What changed for the person between two looks at the fleet (pure), and the "finished" mark that lasts until they see the agent
// (docs/ux/main-window.md, "Finished, not seen yet").
//
// An agent *finishes* when it goes from working to idle or finished (a turn ended, or its program did); one that ended with an error is
// Failed, not finished. Something *needs you* when an agent, or a machine asking to join, comes onto the Needs you list: an agent blocked
// in its terminal or with a request waiting is one thing, however many requests it opens, until it leaves the list. An agent the window
// sees for the first time (the first snapshot, a sign-in again, a new connection) is taken as it is: no sound, no mark.
import { type AgentTileModel, type UiState, endedWithError } from '../model';
import type { AttentionItem } from './fleet';
import type { SoundEvent } from './sounds';

export interface Seen { states: ReadonlyMap<string, UiState>; asking: ReadonlySet<string> }

// Who an item on the Needs you list is about: its agent (a request names it even before the list has the agent), or the machine asking.
export const askerOf = (it: AttentionItem): string => (it.kind === 'approval' ? it.approval.agentId : it.kind === 'agent' ? it.agent.id : it.key);

export interface Changes {
  // Sounds that may play, at most one per agent and change.
  events: SoundEvent[];
  // Agents that finished now.
  finished: string[];
  // Agents at work again or waiting on the person: a mark they had is out of date.
  resumed: string[];
  next: Seen;
}

export function changes(prev: Seen | undefined, agents: readonly AgentTileModel[], attention: readonly AttentionItem[]): Changes {
  const states = new Map<string, UiState>();
  for (const a of agents) states.set(a.id, a.state);
  const asking = new Set<string>();
  const pairs = new Set<string>();
  for (const it of attention) { const who = askerOf(it); asking.add(who); if (it.kind === 'pair') pairs.add(who); }
  const next: Seen = { states, asking };
  if (!prev) return { events: [], finished: [], resumed: [], next };
  const events: SoundEvent[] = [];
  const finished: string[] = [];
  const resumed: string[] = [];
  for (const a of agents) {
    const was = prev.states.get(a.id);
    if (was === undefined || was === a.state) continue;
    if (a.state === 'working' || a.state === 'needs-you') resumed.push(a.id);
    else if (was === 'working' && (a.state === 'idle' || a.state === 'finished') && !endedWithError(a)) {
      finished.push(a.id);
      events.push({ kind: 'finished', agent: a.id });
    }
  }
  for (const who of asking) {
    if (prev.asking.has(who)) continue;
    events.push(pairs.has(who) ? { kind: 'needs-you' } : { kind: 'needs-you', agent: who });
  }
  return { events, finished, resumed, next };
}

// ---- The mark ---------------------------------------------------------------------------------------------------------------------------

// The mark's rules: an agent that finished while the person was not looking at it is marked; looking at it, its working again (or waiting
// on the person), or its leaving the list takes the mark away.
export function nextUnseen(cur: ReadonlySet<string>, c: { finished?: readonly string[]; resumed?: readonly string[]; seen?: ReadonlySet<string>; alive?: ReadonlySet<string> }): ReadonlySet<string> {
  const out = new Set(cur);
  for (const id of c.finished ?? []) if (!c.seen?.has(id)) out.add(id);
  for (const id of c.resumed ?? []) out.delete(id);
  for (const id of out) if (c.seen?.has(id) || (c.alive && !c.alive.has(id))) out.delete(id);
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
