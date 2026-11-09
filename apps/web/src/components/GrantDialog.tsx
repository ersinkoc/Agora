import * as Dialog from '@radix-ui/react-dialog';
import { t } from '../i18n';
import type { GrantKind } from '../net/grant';

// A machine wants the owner's say before this window drives its agents (ADR-0041). Says what it covers and for how long;
// allowing signs it with this device's key.
export function GrantDialog({ machine, kind, busy, failed, onAllow, onClose }: {
  machine: string;
  kind: GrantKind;
  busy?: boolean;
  failed?: boolean;
  onAllow: () => void;
  onClose: () => void;
}) {
  const title = kind === 'typing' ? t('grant-title-typing') : kind === 'start' ? t('grant-title-start') : t('grant-title-other', { machine });
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby="grant-explain" className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-l border border-line bg-surface-1 p-5 shadow-card">
          <Dialog.Title className="m-0 text-base font-semibold tracking-normal text-ink normal-case">{title}</Dialog.Title>
          <p id="grant-explain" className="muted mt-2">{t('grant-explain', { machine })}</p>
          {failed && <p role="alert" className="mt-2">{t('grant-failed')}</p>}
          <div className="mt-4 flex justify-end gap-2">
            <button type="button" className="button" onClick={onClose}>{t('grant-later')}</button>
            <button type="button" className="button button--primary" disabled={busy} onClick={onAllow}>{t('grant-allow')}</button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
