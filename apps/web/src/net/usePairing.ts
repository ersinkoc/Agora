// The owner's side of pairing in the window (ADR-0065): holds the cards in React state (memory only: nothing here touches localStorage,
// sessionStorage, the console or the page title), asks the hub whether this session is the owner's, and sends the owner's answers.
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { PairAnswer } from '../components/PairRequest';
import type { PairingUi } from '../components/Pairing';
import type { Live, LinkState } from './live';
import { applyPairNote, emptyPairState, shownCards, shownRecent, type PairCard, type PairNote, type PairOwner, type PairWindow } from './pairing';
import { ownerOfRole, type PersonRole } from './role';

const code = (e: unknown): string => (e as { code?: string }).code ?? '';
const windowOf = (s: { open?: boolean | null; closesAtMs?: unknown; pending?: number | null } | null | undefined): PairWindow | undefined =>
  s ? { open: !!s.open, closesAtMs: Number(String(s.closesAtMs ?? 0)), pending: s.pending ?? 0 } : undefined;

/** How often the switch's state is read again while the owner has the window open (the hub closes it by itself after 10 minutes). */
export const WINDOW_POLL_MS = 20_000;

export function usePairing(api: Live | undefined, link: LinkState, role: PersonRole = 'unspecified'): { ui: PairingUi | undefined; owner: PairOwner; dispatch: (n: PairNote) => void } {
  const [state, setState] = useState(emptyPairState);
  const dispatch = useCallback((n: PairNote) => setState((s) => applyPairNote(s, n)), []);
  // A new hub connection starts from nothing: what the last session knew is not carried over.
  useEffect(() => { setState(emptyPairState()); }, [api]);

  // Is this session the owner's? The hub answers `PairStatus` to the owner only; everyone else gets `auth.forbidden`. That answer is
  // final for the connection (a change of role or grants ends it, and a new connection asks again), so a window that is not the owner's
  // stops asking: each refusal is a row in the owner's Activity (#904). ADR-0106: a hub that names the role in its `Welcome` is not
  // asked for a member or a viewer at all; the owner's window still asks, to learn whether pairing is on.
  const live = link === 'live';
  const known = ownerOfRole(role);
  useEffect(() => {
    if (!api || !live) return;
    if (known) { dispatch({ type: 'owner', owner: known }); return; }
    let stop = false;
    let timer: ReturnType<typeof setInterval> | undefined;
    const read = () => api.command('', { pairStatus: {} }).then((ok) => {
      if (stop) return;
      dispatch({ type: 'owner', owner: 'yes' });
      dispatch({ type: 'window', window: windowOf(ok.pairStatus) });
    }).catch((e: unknown) => {
      if (stop) return;
      if (code(e) === 'auth.forbidden') { dispatch({ type: 'owner', owner: 'no' }); clearInterval(timer); }
      else if (code(e) === 'protocol.capability_missing') dispatch({ type: 'owner', owner: 'unavailable' });
    });
    void read();
    timer = setInterval(() => { void read(); }, WINDOW_POLL_MS);
    return () => { stop = true; clearInterval(timer); };
  }, [api, live, known, dispatch]);

  const onAnswer = useCallback(async (card: PairCard, approve: boolean, replace: boolean): Promise<PairAnswer> => {
    if (!api) return 'failed';
    try {
      await api.command('', approve ? { pairApprove: { pairId: card.pairId, replace: replace && !!card.nameTaken } } : { pairDeny: { pairId: card.pairId } });
      return 'ok';
    } catch (e) {
      // The hub keeps the request open and says the name already belongs to a machine: the card then asks about Replace.
      const c = e as { code?: string; params?: Record<string, string> };
      if (c.code === 'pair.state' && c.params?.state === 'name_in_use') { dispatch({ type: 'name-taken', id: card.id }); return 'taken'; }
      return 'failed';
    }
  }, [api, dispatch]);
  const onOpen = useCallback(async (): Promise<boolean> => {
    if (!api) return false;
    try { dispatch({ type: 'window', window: windowOf((await api.command('', { pairOpen: {} })).pairStatus) }); return true; } catch { return false; }
  }, [api, dispatch]);
  const onClose = useCallback(async (): Promise<boolean> => {
    if (!api) return false;
    try { await api.command('', { pairClose: {} }); dispatch({ type: 'window', window: { open: false, closesAtMs: 0, pending: 0 } }); return true; } catch { return false; }
  }, [api, dispatch]);

  const ui = useMemo<PairingUi | undefined>(() => (state.owner === 'yes' ? {
    cards: shownCards(state), recent: shownRecent(state, Date.now()), window: state.window, offline: !live, onAnswer, onOpen, onClose,
  } : undefined), [state, live, onAnswer, onOpen, onClose]);
  return { ui, owner: state.owner, dispatch };
}
