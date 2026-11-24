import { useEffect, useState } from 'react';
import { t } from '../i18n';
import { span } from '../net/fleet';
import type { PairCard, PairRecent, PairWindow } from '../net/pairing';
import { PairRequest, type PairAnswer } from './PairRequest';
import { isolated } from './pairChoice';

// What the window gives the console and the phone list to show machines that ask to join (ADR-0065). Built from `PairState` only for an
// owner's session, so a screen that gets one may show it as it is.
export interface PairingUi {
  cards: readonly PairCard[];
  recent: readonly PairRecent[];
  /** The hub's switch; undefined until it has said. */
  window?: PairWindow;
  /** The link to the hub is not live. */
  offline: boolean;
  onAnswer: (card: PairCard, approve: boolean, replace: boolean) => Promise<PairAnswer>;
  /** Opens the 10-minute window / closes it; resolve to whether the hub did it. */
  onOpen?: () => Promise<boolean>;
  onClose?: () => Promise<boolean>;
}

// The full cards, one after the other. Used where there is no queue to put them in (the phone list, a page that is not Home).
export function PairCards({ ui, inline = true }: { ui: PairingUi; inline?: boolean }) {
  return <>{ui.cards.map((c) => <PairRequest key={c.id} inline={inline} request={c} offline={ui.offline} onAnswer={(a, r) => ui.onAnswer(c, a, r)} />)}</>;
}

// "A machine named X was approved": one line each, a little while after the request ended. The name is what the machine called itself.
// Messages are formatted without Fluent's own isolation, so the name goes in between Unicode isolates (`isolated`) and cannot reorder
// the words around it.
export function PairRecentList({ recent }: { recent: readonly PairRecent[] }) {
  if (recent.length === 0) return null;
  return (
    <ul className="pair-recent" aria-label={t('pair-recent')}>
      {recent.map((r) => <li key={r.id + r.at} className="pair-recent__line">{t(`pair-result-${r.outcome}`, { name: isolated(r.name) })}</li>)}
    </ul>
  );
}

// The hub's switch for new machines: closed, with a button to open it for 10 minutes; or open, with the time left and a Close button.
export function PairWindowControl({ ui, now: clock, intro }: { ui: PairingUi; now?: number; intro?: boolean }) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const [ticked, setTicked] = useState(() => Date.now());
  const open = !!ui.window?.open;
  useEffect(() => {
    if (!open || clock !== undefined) return;
    const timer = setInterval(() => setTicked(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [open, clock]);
  const now = clock ?? ticked;
  const run = async (f: (() => Promise<boolean>) | undefined) => {
    if (!f) return;
    setBusy(true);
    setFailed(!(await f().catch(() => false)));
    setBusy(false);
  };
  const left = ui.window ? Math.max(0, ui.window.closesAtMs - now) : 0;
  const reason = ui.offline ? t('pair-offline') : undefined;
  if (!ui.onOpen && !ui.onClose) return null;
  return (
    <div className="pair-window" role="group" aria-label={t('pair-window-label')}>
      {open ? <>
        <p className="m-0 text-sm text-ink" role="status">{t('pair-window-open', { time: span(left) })}</p>
        <p className="m-0 text-xs text-muted">{t('pair-window-open-help')}</p>
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" className="button" disabled={busy || ui.offline} onClick={() => void run(ui.onClose)}>{t('pair-window-close')}</button>
          {reason && <span className="text-xs text-muted">{reason}</span>}
        </div>
      </> : <>
        {intro && <p className="m-0 text-xs text-muted">{t('pair-window-help')}</p>}
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" className="button button--primary" disabled={busy || ui.offline || !ui.onOpen} onClick={() => void run(ui.onOpen)}>{t('pair-window-allow')}</button>
          {reason && <span className="text-xs text-muted">{reason}</span>}
        </div>
      </>}
      {failed && <p role="alert" className="m-0 text-xs text-danger">{t('pair-window-failed')}</p>}
    </div>
  );
}
