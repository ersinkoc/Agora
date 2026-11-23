import * as Dialog from '@radix-ui/react-dialog';
import { useState } from 'react';
import { t } from '../i18n';
import { forgetMachineReasonKey } from '../net/forget';
import type { MachineModel } from '../net/view';
import { Icon } from './Icon';
import { Button, Pill } from './kit';
import { MachineFacts, MachineName, OsTile } from './machineFacts';
import { isolated } from './pairChoice';

// ADR-0073: forget one machine, or clean up every old one at once. Each machine is listed with when it was last seen and what was
// running there then (ticked, in a clean-up); one more box also refuses them if they connect again (they would need a new join code).
// The answer for each machine is its error code, or undefined when the hub forgot it. A destructive dialog (ADR-0102): a danger icon
// chip, the safe answer first and focused, and the action itself in red, named for what it forgets.
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
  // A row is a label only when it has a box to tick (a clean-up); one machine is simply shown.
  const Row = cleanup ? 'label' : 'div';
  const one = !cleanup && machines.length === 1 ? machines[0] : undefined;
  const title = one ? t('forget-machine-title', { name: isolated(one.name) }) : t('forget-machines-title', { count: machines.length });
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content role="alertdialog" aria-describedby="forget-machines-explain"
          onOpenAutoFocus={(e) => { e.preventDefault(); (e.currentTarget as HTMLElement).querySelector<HTMLElement>('[data-keep]')?.focus(); }}
          className="kit-dialog fixed left-1/2 top-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] max-h-[calc(100vh-2rem)] -translate-x-1/2 -translate-y-1/2 overflow-auto">
          <div className="flex items-start gap-3">
            <span aria-hidden className="kit-chip kit-tone-danger"><Icon name="trash" /></span>
            <div className="grid min-w-0 gap-1">
              <Dialog.Title className="m-0 text-base font-semibold tracking-normal text-ink normal-case">{title}</Dialog.Title>
              <p id="forget-machines-explain" className="muted m-0">{t(one ? 'forget-machine-explain' : 'forget-machines-explain')}</p>
            </div>
          </div>
          <ul className="m-0 grid list-none gap-2 p-0">
            {machines.map((m) => (
              <li key={m.id}>
                <Row className="forget-row">
                  {cleanup && <input type="checkbox" checked={chosen.has(m.id)} onChange={() => toggle(m.id)} />}
                  <OsTile os={m.os} />
                  <span className="grid min-w-0 flex-1">
                    <MachineName m={m} />
                    <span className="text-xs text-muted">
                      <MachineFacts m={m} now={now} />
                      {m.agentsRunning > 0 ? ` · ${t('forget-machine-agents', { count: m.agentsRunning })}` : ''}
                    </span>
                    {failed.has(m.id) && <span role="alert" className="text-xs text-danger">{t(forgetMachineReasonKey(failed.get(m.id) ?? ''))}</span>}
                  </span>
                  <Pill>{t('machine-offline')}</Pill>
                </Row>
              </li>
            ))}
          </ul>
          <label className="forget-row forget-row--check">
            <input type="checkbox" checked={refuse} onChange={(e) => setRefuse(e.target.checked)} />
            <span className="grid min-w-0">
              <strong className="font-semibold">{t(one ? 'forget-machine-refuse' : 'forget-machines-refuse-title')}</strong>
              <span className="text-xs text-muted">{t(one ? 'forget-machine-refuse-hint' : 'forget-machines-refuse-hint')}</span>
            </span>
          </label>
          <div className="flex flex-wrap justify-end gap-2">
            <Button data-keep onClick={onClose}>{t('forget-machines-cancel')}</Button>
            <Button variant="danger" disabled={busy || chosen.size === 0} onClick={forget}>
              {one ? t('forget-machine-confirm-one', { name: isolated(one.name) }) : t('forget-machines-confirm', { count: chosen.size })}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
