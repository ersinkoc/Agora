import { createContext, useContext, useState } from 'react';
import { t } from '../i18n';
import { handoffSettled, type HandoffDeliveryModel } from '../net/view';
import { OWNER_MAX, plain } from './boardColumns';

export const AcknowledgeHandoffContext = createContext<((id: string) => Promise<void>) | undefined>(undefined);

function Delivery({ delivery }: { delivery: HandoffDeliveryModel }) {
  const acknowledge = useContext(AcknowledgeHandoffContext);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  async function clearReservation() {
    if (!acknowledge || busy) return;
    setBusy(true);
    setFailed(false);
    try {
      await acknowledge(delivery.id);
    } catch {
      setFailed(true);
      setBusy(false);
    }
  }

  return (
    <li className="grid min-w-0 gap-1 text-xs text-muted">
      <span className="break-words">{t(
        delivery.acknowledged ? 'board-handoff-acknowledged' : `board-handoff-${delivery.state}`,
        { name: plain(delivery.receiver, OWNER_MAX) },
      )}</span>
      {delivery.errorCode && <code dir="ltr" className="break-words font-mono">{plain(delivery.errorCode, 64)}</code>}
      {delivery.state === 'unresolved' && !delivery.acknowledged && acknowledge &&
        <button type="button" className="kit-btn kit-btn--sm w-fit" disabled={busy}
          onClick={() => void clearReservation()}>{t('board-handoff-acknowledge')}</button>}
      {failed && delivery.state === 'unresolved' && !delivery.acknowledged && <span role="alert">{t('board-handoff-acknowledge-failed')}</span>}
    </li>
  );
}

export function BoardHandoffs({ deliveries }: { deliveries: readonly HandoffDeliveryModel[] }) {
  if (!deliveries.length) return null;
  const active = deliveries.filter((delivery) => !handoffSettled(delivery));
  const history = deliveries.filter(handoffSettled);
  if (!active.length && history[0]) active.push(history.shift()!);
  return (
    <div className="grid min-w-0 gap-1 border-t border-line pt-1.5">
      {active.length > 0 && <ul className="m-0 grid list-none gap-1.5 p-0">
        {active.map((delivery) => <Delivery key={delivery.id} delivery={delivery} />)}
      </ul>}
      {history.length > 0 && <details className="text-xs text-muted">
        <summary className="cursor-pointer">{t('board-handoff-history')}</summary>
        <ul className="m-0 mt-1 grid list-none gap-1.5 p-0">
          {history.map((delivery) => <Delivery key={delivery.id} delivery={delivery} />)}
        </ul>
      </details>}
    </div>
  );
}
