import * as Dialog from '@radix-ui/react-dialog';
import { t } from '../i18n';
import type { GrantKind } from '../net/grant';

// A machine wants the owner's say before this window drives its agents (ADR-0041). Says what it covers and for how long;
// allowing signs it with this device's key.
export function GrantDialog({ machine, agent, kind, busy, failed, reason, onAllow, onClose }: {
  machine: string;
  agent: string;
  kind: GrantKind;
  busy?: boolean;
  failed?: boolean;
  /** Why it failed (a locale key from `grantFailureKey`); the plain failure when absent. */
  reason?: string;
  onAllow: () => void;
  onClose: () => void;
}) {
  const title = kind === 'start' ? t('grant-title-start') : kind === 'other' ? t('grant-title-other', { machine }) : t(`grant-title-${kind}`, { agent });
  return (
    // While Allow is under way the answer is already being sent: Not now, Escape and a click outside do nothing until it is done.
    <Dialog.Root open onOpenChange={(open) => { if (!open && !busy) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby="grant-explain" aria-busy={busy || undefined} onEscapeKeyDown={(e) => { if (busy) e.preventDefault(); }}
          onPointerDownOutside={(e) => { if (busy) e.preventDefault(); }} onInteractOutside={(e) => { if (busy) e.preventDefault(); }} className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-l border border-line bg-surface-1 p-5 shadow-card">
          <Dialog.Title className="m-0 text-base font-semibold tracking-normal text-ink normal-case">{title}</Dialog.Title>
          <p id="grant-explain" className="muted mt-2">{t('grant-explain', { machine })}</p>
          {failed && <p role="alert" className="mt-2">{t(reason ?? 'grant-failed')}</p>}
          <div className="mt-4 flex justify-end gap-2">
            <button type="button" className="button" disabled={busy} onClick={onClose}>{t('grant-later')}</button>
            <button type="button" className="button button--primary" disabled={busy} onClick={onAllow}>{busy ? t('grant-allowing') : t('grant-allow')}</button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
