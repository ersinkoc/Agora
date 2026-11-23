import * as Dialog from '@radix-ui/react-dialog';
import { useState } from 'react';
import { t } from '../i18n';
import { forgetMachineReasonKey } from '../net/forget';
import { ago } from '../net/time';
import type { MachineModel } from '../net/view';

// ADR-0073: forget one machine, or clean up every old one at once. Each machine is listed with when it was last seen and what was
// running there then, ticked; one more box also refuses them if they connect again (they would need a new join code). The answer
// for each machine is its error code, or undefined when the hub forgot it.
export function ForgetMachines({ machines, now, cleanup, onForget, onClose }: {
  machines: readonly MachineModel[];
  now: number;
  cleanup: boolean;
  onForget: (ids: string[], refuseReturn: boolean) => Promise<Map<string, string | undefined>>;
  onClose: () => void;
}) {
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(machines.map((m) => m.id)));
  const [refuse, setRefuse] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<Map<string, string>>(new Map());
  const toggle = (id: string) => setChosen((c) => { const n = new Set(c); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  const forget = async () => {
    setBusy(true);
    const answers = await onForget(machines.filter((m) => chosen.has(m.id)).map((m) => m.id), refuse);
    setBusy(false);
    const left = new Map([...answers].filter((e): e is [string, string] => e[1] !== undefined));
    if (left.size === 0) onClose(); else setFailed(left);
  };
  const title = cleanup ? t('forget-machines-title', { count: machines.length }) : t('forget-machine-title', { name: machines[0]?.name ?? '' });
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby="forget-machines-explain" className="fixed left-1/2 top-1/2 z-50 grid w-[min(30rem,calc(100vw-2rem))] max-h-[calc(100vh-2rem)] -translate-x-1/2 -translate-y-1/2 gap-3 overflow-auto rounded-l border border-line bg-surface-1 p-5 shadow-card">
          <Dialog.Title className="m-0 text-base font-semibold tracking-normal text-ink normal-case">{title}</Dialog.Title>
          <p id="forget-machines-explain" className="muted m-0">{t('forget-machines-explain')}</p>
          <ul className="m-0 grid list-none gap-2 p-0">
            {machines.map((m) => (
              <li key={m.id}>
                <label className="flex items-start gap-2">
                  {cleanup && <input type="checkbox" checked={chosen.has(m.id)} onChange={() => toggle(m.id)} className="mt-1" />}
                  <span className="grid min-w-0">
                    <strong className="truncate">{m.name}</strong>
                    <span className="text-xs text-muted">
                      {t('network-last-seen', { when: ago(m.lastSeenMs, now) })}
                      {m.agentsRunning > 0 ? ` · ${t('forget-machine-agents', { count: m.agentsRunning })}` : ''}
                    </span>
                    {failed.has(m.id) && <span role="alert" className="text-xs text-danger">{t(forgetMachineReasonKey(failed.get(m.id) ?? ''))}</span>}
                  </span>
                </label>
              </li>
            ))}
          </ul>
          <label className="flex items-start gap-2 text-[0.8125rem]">
            <input type="checkbox" checked={refuse} onChange={(e) => setRefuse(e.target.checked)} className="mt-1" />
            <span>{t('forget-machines-refuse')}</span>
          </label>
          <div className="flex justify-end gap-2">
            <button type="button" className="button" onClick={onClose}>{t('forget-machines-cancel')}</button>
            <button type="button" className="button button--primary" disabled={busy || chosen.size === 0} onClick={forget}>
              {t('forget-machines-confirm', { count: chosen.size })}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
