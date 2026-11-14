import { useState } from 'react';
import { t } from '../i18n';
import { Icon } from './Icon';

export interface DeviceRequestModel {
  id: string;
  name: string;
  fingerprint: string;
}

// ADR-0061: another device of the same person asks this one, which machines already trust, to approve it. The card names it and its
// fingerprint (what the person compares with the one that device shows) and the machines the approval is for; approving signs it with
// this device's key, and each machine checks the signature itself.
export function DeviceRequest({ request, machines, onApprove, onLater, inline }: {
  request: DeviceRequestModel;
  // Shown in place (the gallery) instead of floating over the window.
  inline?: boolean;
  // The machines that trust this device: the only ones where its approval counts. Names to show, ids to sign.
  machines: readonly { id: string; name: string }[];
  // `nodes` empty: every machine of the person (also ones that come to trust this device later).
  onApprove: (nodes: string[]) => Promise<boolean>;
  onLater: () => void;
}) {
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(machines.map((m) => m.id)));
  const [state, setState] = useState<'idle' | 'busy' | 'failed'>('idle');
  const all = machines.every((m) => chosen.has(m.id));
  const approve = async () => {
    setState('busy');
    const ok = await onApprove(all ? [] : machines.filter((m) => chosen.has(m.id)).map((m) => m.id));
    setState(ok ? 'idle' : 'failed');
  };
  return (
    <section role="dialog" aria-labelledby={`device-request-${request.id}`}
      className={`${inline ? '' : 'fixed bottom-4 end-4 z-40 '}grid w-[min(24rem,calc(100vw-2rem))] gap-3 rounded-m border border-line bg-surface-1 p-4 shadow-card`}>
      <div className="flex items-start gap-2">
        <Icon name="monitor" className="mt-0.5 size-4 text-accent" />
        <h2 id={`device-request-${request.id}`} className="m-0 flex-1 text-sm font-semibold tracking-normal text-ink normal-case">{t('device-request-title', { name: request.name || request.id })}</h2>
      </div>
      <div className="grid gap-1">
        <span className="text-xs text-muted">{t('device-fingerprint')}</span>
        <span className="font-mono text-base tracking-wide text-ink">{request.fingerprint}</span>
        <span className="text-xs text-muted">{t('device-request-compare')}</span>
      </div>
      <fieldset className="m-0 grid gap-1 border-0 p-0">
        <legend className="mb-1 p-0 text-xs text-muted">{t('device-request-machines')}</legend>
        {machines.map((m) => (
          <label key={m.id} className="flex items-center gap-2 text-[0.8125rem]">
            <input type="checkbox" checked={chosen.has(m.id)} disabled={state === 'busy'}
              onChange={(e) => setChosen((c) => { const n = new Set(c); if (e.target.checked) n.add(m.id); else n.delete(m.id); return n; })} />
            {m.name}
          </label>
        ))}
        <span className="text-xs text-muted">{all ? t('device-request-all') : t('device-request-some')}</span>
      </fieldset>
      {state === 'failed' && <p role="alert" className="m-0 text-needs-you">{t('device-request-failed')}</p>}
      <div className="flex justify-end gap-2">
        <button type="button" className="button" disabled={state === 'busy'} onClick={onLater}>{t('device-request-later')}</button>
        <button type="button" className="button button--primary" disabled={state === 'busy' || chosen.size === 0} onClick={() => void approve()}>{t('device-request-approve')}</button>
      </div>
    </section>
  );
}
