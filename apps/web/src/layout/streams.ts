// Which agent each pane shows, and which panes hold a live terminal stream. Only panes on screen subscribe (ADR-0068 §7,
// ADR-0022): a background tab, a pane hidden behind a maximized one, or every pane but the front one on a phone holds none.
import type { TerminalSession, TerminalSource } from '../terminal/source';
import { type Workspace, visiblePanes } from './engine';
import { type Pane, bindOf, followOf } from './tree';

export interface Bindings {
  selection?: string; // the agent selected in the list
  attention: ReadonlyMap<string, string>; // pane id → the agent that pane follows for "needs you"
}

export function agentOf(pane: Pane, b: Bindings): string | undefined {
  const f = followOf(pane);
  if (f === 'selection') return b.selection;
  if (f === 'attention') return b.attention.get(pane.id);
  return bindOf(pane);
}

// Panes that follow attention each show a different agent that needs you, oldest first. A pane keeps its agent while that agent
// still needs you (it never jumps away mid-read); when it stops, the pane moves on to the next one nobody shows.
export function assignAttention(panes: readonly string[], prev: ReadonlyMap<string, string>, waiting: readonly string[]): Map<string, string> {
  const out = new Map<string, string>();
  const taken = new Set<string>();
  for (const p of panes) {
    const a = prev.get(p);
    if (a && waiting.includes(a) && !taken.has(a)) { out.set(p, a); taken.add(a); }
  }
  const free = waiting.filter((a) => !taken.has(a));
  for (const p of panes) {
    if (out.has(p)) continue;
    const a = free.shift();
    if (a) out.set(p, a);
  }
  return out;
}

export const sameAssignment = (a: ReadonlyMap<string, string>, b: ReadonlyMap<string, string>) =>
  a.size === b.size && [...a].every(([k, v]) => b.get(k) === v);

// Pane id → agent id, for every pane on screen that draws a terminal.
export function streamPlan(ws: Workspace, narrow: boolean, b: Bindings, hasTerminal: (agent: string) => boolean): Map<string, string> {
  const plan = new Map<string, string>();
  for (const p of visiblePanes(ws, narrow)) {
    if (p.panel !== 'terminal' && p.panel !== 'agent') continue;
    const a = agentOf(p, b);
    if (a && hasTerminal(a)) plan.set(p.id, a);
  }
  return plan;
}

// The live terminal streams, kept in step with a plan the way the window mounts terminals: one per pane, keyed by pane and agent,
// so a pane that keeps its agent keeps its stream (a resize, a move, a preset that keeps it) and anything else is closed.
export class TerminalStreams {
  private open = new Map<string, TerminalSession>();
  constructor(private sourceFor: (agent: string) => TerminalSource) {}

  sync(plan: ReadonlyMap<string, string>) {
    const want = new Map([...plan].map(([pane, agent]) => [`${pane}:${agent}`, agent]));
    for (const [key, session] of this.open) if (!want.has(key)) { session.close(); this.open.delete(key); }
    for (const [key, agent] of want) if (!this.open.has(key)) this.open.set(key, this.sourceFor(agent).connect(() => {}));
  }

  get size() { return this.open.size; }
  close() { this.sync(new Map()); }
}
