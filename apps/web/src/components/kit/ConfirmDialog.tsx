import * as Dialog from '@radix-ui/react-dialog';
import type { ReactNode } from 'react';
import { Icon, type IconName } from '../Icon';
import { Button } from './Button';

// A question before something that cannot be taken back (Remove a device, Forget a machine): an overlay on the raised surface, a red icon
// chip, a title that names the thing, one line on what follows, then the safe answer first and the destructive one in red (ADR-0102).
// The safe answer is the first control, so it has the focus when the dialog opens; Escape, a click outside and the safe answer all close
// it without calling `onConfirm`. Only the red button does.
export function ConfirmDialog({ open, icon = 'trash', title, body, cancel, confirm, onCancel, onConfirm }: {
  open: boolean; icon?: IconName; title: ReactNode; body: ReactNode; cancel: string; confirm: string; onCancel: () => void; onConfirm: () => void;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={(o) => { if (!o) onCancel(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="kit-overlay" />
        <Dialog.Content className="kit-dialog" data-danger="">
          <span aria-hidden className="kit-chip kit-tone-danger"><Icon name={icon} /></span>
          <Dialog.Title className="kit-dialog__title">{title}</Dialog.Title>
          <Dialog.Description className="kit-dialog__body">{body}</Dialog.Description>
          <div className="kit-dialog__actions">
            <Button onClick={onCancel}>{cancel}</Button>
            <Button variant="danger" onClick={onConfirm}>{confirm}</Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
