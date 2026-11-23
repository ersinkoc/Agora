import { createContext, createElement, useCallback, useContext, useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { ATTENTION_DWELL_MS } from '../net/attention';
import type { ApprovalModel } from '../net/view';

// T44 for every list of requests that can be answered in place: Home's queue, the side panel's cards, and the cards above a terminal and
// above a chat's composer (#1045). One mechanism, moved here from Home's queue (PR 2, after Codex's reviews of #1028) so each surface keeps
// exactly the same rules:
// - Allow and Deny do nothing for a short dwell (ATTENTION_DWELL_MS, as the attention dialog and the phone bar) after anything the list
//   shows changed (a card came, went, moved, or says something else: a card that moves can put a ready Allow under a click meant for
//   another), after the panes were rearranged (`Arrangement`: a move, a split, a resize, another tab in front), after the person came back
//   to the window, and after the link came back. One timer per surface, derived from what is shown (`layoutOf`), so no frame has a new list
//   ready. The dwell is the surface's own.
// - Answers are held by request (its id: stable across layouts, focus and the dwell), for the whole window (#1048): one `AnswerHolds`
//   that every surface's guard, the attention dialog and a switch's card read and write, so a request answered or on its way on one
//   surface is held on all of them, and a surface that closes and opens again (the side panel) finds its holds where they were. Two sets,
//   each with its own way out:
//   - `inFlight`: from the accepted press until that answer's own callback settles. Nothing else releases it: not a new layout, not the
//     request leaving the list and coming back, not a surface closing, not the person coming back to the window.
//   - `sent`: an answer that went (the window signed it and the machine took it). The card says "Answer sent" and never asks for a second
//     answer while the request is listed (T44: answered once; a card that stays is the list catching up). Forgotten when it leaves the
//     window's list of requests (`AnswerHoldsProvider`'s `listed`), never because one surface stopped drawing it.
//   A failed answer (`false`, or a promise that rejects, or a callback that throws) is in neither, so the request is answerable again after
//   a fresh dwell: every surface that lists it starts its dwell again when an answer settles. Each request's status is offered to
//   `layoutOf`, so a list that draws it (a status line, a Review button) restarts the dwell too. Other requests stay answerable meanwhile.
//   A guard drawn outside a provider (a card alone, a test) keeps holds of its own, pruned by its own list.
// The click handler and the buttons ask the same question, `canAnswer`, which reads the holds at the moment of the press.

/** What an answer callback returns: `false` (or a promise of it, or one that rejects) is a failed answer; anything else went. */
export type AnswerResult = void | boolean | PromiseLike<void | boolean>;
export type AnswerStatus = 'open' | 'held' | 'sent';
export interface AnswerGuard {
  /** Whether a press on this request's answers would send now. */
  canAnswer: (id: string) => boolean;
  /** Sends `send()` for this request if it can be answered now, and holds the request until the result settles. */
  answer: (id: string, send: () => unknown) => void;
  status: (id: string) => AnswerStatus;
  /** Something moved that the layout key cannot see (a pane narrow enough to put the buttons under the text): the dwell starts again. */
  restart: () => void;
  /** The link is not live: nothing can be answered, and the surface says why. */
  stale: boolean;
  /** The holds this guard reads and writes (the window's, or its own outside a provider), for what it opens (`ShareHolds`). */
  holds: AnswerHolds;
}

/** What a card draws of one request, for a surface's layout key: a change in any of it restarts the dwell. */
export const requestShown = (x: ApprovalModel, agent?: { name?: string; accent?: string; kind?: string }): unknown[] =>
  [x.id, x.agentId, x.agentName, x.machineName ?? '', x.action, x.detail, x.always ?? null, !!x.start, !!x.skip, !!x.skipUnknown, agent?.name ?? '', agent?.accent ?? '', agent?.kind ?? ''];

/** The answers this window holds, by request id (#1048). A plain object with a subscription, so a press reads it at once (no render
 *  needed) and every surface draws from the same sets. */
export class AnswerHolds {
  private readonly inFlight = new Set<string>();
  private readonly sent = new Set<string>();
  private listed: ReadonlySet<string> = new Set();
  private version = 0;
  private readonly subs = new Set<() => void>();
  private readonly settles = new Set<(id: string) => void>();
  private notify() { this.version += 1; for (const s of [...this.subs]) s(); }
  readonly subscribe = (fn: () => void) => { this.subs.add(fn); return () => { this.subs.delete(fn); }; };
  readonly snapshot = () => this.version;
  /** Called with a request's id whenever an answer to it settles (went or failed). */
  readonly onSettle = (fn: (id: string) => void) => { this.settles.add(fn); return () => { this.settles.delete(fn); }; };
  readonly status = (id: string): AnswerStatus => (this.sent.has(id) ? 'sent' : this.inFlight.has(id) ? 'held' : 'open');
  /** The requests that are listed now. An answer that went is forgotten once its request is not; this runs during render, so it never
   *  notifies (nothing that is drawn can show a request nobody lists). */
  list(ids: ReadonlySet<string>) {
    this.listed = ids;
    for (const id of this.sent) if (!ids.has(id)) this.sent.delete(id);
  }
  /** Holds `id` and sends `send()`, unless the request is already held or answered; false when it sent nothing. */
  hold(id: string, send: () => unknown): boolean {
    if (this.status(id) !== 'open') return false;
    this.inFlight.add(id);
    this.notify();
    const done = (v: unknown) => {
      this.inFlight.delete(id);
      if (v !== false && this.listed.has(id)) this.sent.add(id);
      this.notify();
      for (const fn of [...this.settles]) fn(id);
    };
    let result: unknown;
    try { result = send(); } catch { result = false; }
    if (result && typeof (result as PromiseLike<unknown>).then === 'function') (result as PromiseLike<unknown>).then(done, () => done(false));
    else done(result);
    return true;
  }
}

const HoldsContext = createContext<AnswerHolds | undefined>(undefined);

/** One set of holds for everything under it (the window: `App`). `listed`: the id of every request the window knows of now. */
export function AnswerHoldsProvider({ listed, children }: { listed: readonly string[]; children?: ReactNode }) {
  const [holds] = useState(() => new AnswerHolds());
  holds.list(new Set(listed));
  return createElement(HoldsContext.Provider, { value: holds }, children);
}

/** Gives everything under it these holds: what a card opens (its Review dialog) holds a request with that card's guard, also outside
 *  the window's provider (a card alone, a test). */
export function ShareHolds({ holds, children }: { holds: AnswerHolds; children?: ReactNode }) {
  return createElement(HoldsContext.Provider, { value: holds }, children);
}

/** The window's holds, or (outside a provider) holds of this component's own, pruned by `listed`. */
export function useAnswerHolds(listed: readonly string[]): AnswerHolds {
  const shared = useContext(HoldsContext);
  const [own] = useState(() => new AnswerHolds());
  if (!shared) own.list(new Set(listed));
  const holds = shared ?? own;
  useSyncExternalStore(holds.subscribe, holds.snapshot, holds.snapshot);
  return holds;
}

/** Where the panes are drawn now (a move, a split, a resize, another tab in front), as a string: a change restarts every dwell under it.
 *  Never which pane has the focus: a click that focuses a pane must not hold back the Allow it lands on. */
export const Arrangement = createContext('');

// `listed`: the ids of every request the surface lists (shown or not), for forgetting answers that went. `layoutOf`: everything the surface
// draws that can move or change a card, as a string, given each request's status.
export function useAnswerGuard(listed: readonly string[], layoutOf: (status: (id: string) => AnswerStatus) => string, stale = false): AnswerGuard {
  const holds = useAnswerHolds(listed);
  const arrangement = useContext(Arrangement);
  const shown = useRef<ReadonlySet<string>>(new Set());
  shown.current = new Set(listed);
  const { status } = holds;
  const layout = JSON.stringify([stale, arrangement, layoutOf(status)]);

  // Readiness is a ref, checked in the click handler itself, so it is right even before React has drawn the change: `ready` holds the
  // layout it was earned for, and each timer only sets it for the generation it was started in. A new layout (seen in render), the person
  // coming back to the window (focus, or the tab shown again) and the link changing all clear it at once and start a new generation.
  const ready = useRef<string | undefined>(undefined);
  const generation = useRef(0);
  const seenLayout = useRef(layout);
  const [, redraw] = useState(0);
  if (seenLayout.current !== layout) { seenLayout.current = layout; ready.current = undefined; generation.current += 1; }
  const [epoch, setEpoch] = useState(0);
  const restart = useCallback(() => { ready.current = undefined; generation.current += 1; setEpoch((n) => n + 1); }, []);
  useEffect(() => {
    const seen = () => { if (document.visibilityState === 'visible') restart(); };
    window.addEventListener('focus', restart);
    document.addEventListener('visibilitychange', seen);
    return () => { window.removeEventListener('focus', restart); document.removeEventListener('visibilitychange', seen); };
  }, [restart]);
  useEffect(() => {
    const mine = generation.current;
    const h = setTimeout(() => { if (generation.current === mine) { ready.current = layout; redraw((n) => n + 1); } }, ATTENTION_DWELL_MS);
    return () => clearTimeout(h);
  }, [layout, epoch]);

  // An answer to a request this surface lists settled (here or on another surface): what its card shows changed, so the dwell starts again
  // (also for a failure that came back before any redraw, and on a surface whose layout does not draw the status).
  useEffect(() => holds.onSettle((id) => { if (shown.current.has(id)) restart(); }), [holds, restart]);

  const canAnswer = (id: string) => !stale && ready.current === layout && holds.status(id) === 'open';
  // The guard is here as well as on the buttons: a press during the dwell, on a held or answered request (on any surface), or from an older
  // drawing sends nothing.
  const answer = (id: string, send: () => unknown) => {
    if (!canAnswer(id)) return;
    holds.hold(id, send);
  };
  return { canAnswer, answer, status, restart, stale, holds };
}
