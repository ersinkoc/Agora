import { useCallback, useEffect, useRef, useState } from 'react';
import { ATTENTION_DWELL_MS } from '../net/attention';
import type { ApprovalModel } from '../net/view';

// T44 for every list of requests that can be answered in place: Home's queue, the side panel's cards, and the cards above a terminal and
// above a chat's composer (#1045). One mechanism, moved here from Home's queue (PR 2, after Codex's reviews of #1028) so each surface keeps
// exactly the same rules:
// - Allow and Deny do nothing for a short dwell (ATTENTION_DWELL_MS, as the attention dialog and the phone bar) after anything the list
//   shows changed (a card came, went, moved, or says something else: a card that moves can put a ready Allow under a click meant for
//   another), after the person came back to the window, and after the link came back. One timer for the whole list, derived from what is
//   shown (`layoutOf`), so no frame has a new list ready.
// - Answers are held by request (its id: stable across layouts, focus and the dwell), in two sets, each with its own way out:
//   - `inFlight`: from the accepted press until that answer's own callback settles. Nothing else releases it: not a new layout, not the
//     request leaving the list and coming back, not the person coming back to the window.
//   - `sent`: an answer that went (the window signed it and the machine took it). The card says "Answer sent" and never asks for a second
//     answer while the request is listed (T44: answered once; a card that stays is the list catching up). Forgotten when it leaves the list.
//   A failed answer (`false`, or a promise that rejects, or a callback that throws) is in neither, so the request is answerable again after
//   a fresh dwell. Each request's status is offered to `layoutOf`, so a list that draws it (a status line, a Review button) restarts the
//   dwell when an answer settles. Other requests stay answerable meanwhile.
// The click handler and the buttons ask the same question, `canAnswer`.

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
}

/** What a card draws of one request, for a surface's layout key: a change in any of it restarts the dwell. */
export const requestShown = (x: ApprovalModel, agent?: { name?: string; accent?: string; kind?: string }): unknown[] =>
  [x.id, x.agentId, x.agentName, x.machineName ?? '', x.action, x.detail, x.always ?? null, !!x.start, !!x.skip, !!x.skipUnknown, agent?.name ?? '', agent?.accent ?? '', agent?.kind ?? ''];

// `listed`: the ids of every request the surface lists (shown or not), for forgetting answers that went. `layoutOf`: everything the surface
// draws that can move or change a card, as a string, given each request's status.
export function useAnswerGuard(listed: readonly string[], layoutOf: (status: (id: string) => AnswerStatus) => string, stale = false): AnswerGuard {
  const inFlight = useRef(new Set<string>());
  const sent = useRef(new Set<string>());
  const shownIds = useRef(new Set<string>());
  shownIds.current = new Set(listed);
  for (const id of sent.current) if (!shownIds.current.has(id)) sent.current.delete(id);
  const status = (id: string): AnswerStatus => (sent.current.has(id) ? 'sent' : inFlight.current.has(id) ? 'held' : 'open');
  const layout = JSON.stringify([stale, layoutOf(status)]);

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

  const canAnswer = (id: string) => !stale && ready.current === layout && !inFlight.current.has(id) && !sent.current.has(id);
  // The guard is here as well as on the buttons: a press during the dwell, on a held or answered request, or from an older drawing sends nothing.
  const answer = (id: string, send: () => unknown) => {
    if (!canAnswer(id)) return;
    inFlight.current.add(id);
    redraw((n) => n + 1);
    const done = (v: unknown) => {
      inFlight.current.delete(id);
      if (v !== false && shownIds.current.has(id)) sent.current.add(id);
      // Either way what the card shows changed, and the dwell starts again (also for a failure that came back before any redraw).
      restart();
    };
    let result: unknown;
    try { result = send(); } catch { result = false; }
    if (result && typeof (result as PromiseLike<unknown>).then === 'function') (result as PromiseLike<unknown>).then(done, () => done(false));
    else done(result);
  };
  return { canAnswer, answer, status, restart, stale };
}
