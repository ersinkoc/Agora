import { t } from '../i18n';
import { approvalName } from '../net/approvalText';
import { useSkipCapable } from '../net/skipApproval';
import { visible, type ApprovalModel } from '../net/view';
import { StateMark } from './StateBadge';

// ADR-0090 Amendment A: a window asked to switch a running agent to a mode or option in which it acts without asking, and the machine
// wants the owner's signed answer first. Every field is the machine's own rendering of the request (`detail`), shown as it gave it with
// any hidden character written as a visible escape: the agent's names for the setting and the choice are its own words, so the ids that
// the answer is signed over are shown beside them. Nothing here is rebuilt from the window's own option labels.
// The card is answerable only for a machine the window has heard offer this (`approval.skip_permissions`) and only in the shape it expects;
// any other shape says to update the window and signs nothing.
export function SkipApproval({ approval, busy, onResolve }: {
  approval: ApprovalModel;
  busy?: boolean;
  onResolve?: (approved: boolean) => void;
}) {
  const capable = useSkipCapable(approval.node);
  const skip = approval.skip;
  if (!skip) {
    return (
      <article role="alert" aria-label={t('skip-approval-unknown-title')} className="flex flex-col gap-1 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
        <div className="flex items-center gap-2"><StateMark state="needs-you" /><strong className="min-w-0 flex-1 break-words">{t('skip-approval-unknown-title')}</strong></div>
        <p className="m-0 text-xs text-muted">{t('skip-approval-unknown')}</p>
      </article>
    );
  }
  const who = visible(approvalName(approval));
  const title = t('skip-approval-title', { name: who });
  const answerable = capable && !!onResolve;
  const setting = skip.optionType === 'mode' ? t('session-mode') : visible(skip.optionName) || visible(skip.option);
  return (
    <article role="alert" aria-label={title} data-skip-approval className="flex flex-col gap-2 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
      <div className="flex items-center gap-2">
        <StateMark state="needs-you" />
        <strong className="min-w-0 flex-1 break-words">{title}</strong>
      </div>
      <p className="m-0 text-xs">{t('skip-approval-body')}</p>
      <dl className="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        {approval.machineName?.trim() && <><dt className="text-muted">{t('skip-approval-machine')}</dt><dd className="m-0 break-words">{visible(approval.machineName)}</dd></>}
        <dt className="text-muted">{t('start-approval-folder')}</dt>
        <dd className="m-0 break-all font-mono" dir="ltr">{visible(skip.folder)}</dd>
        <dt className="text-muted">{t('skip-approval-setting')}</dt>
        <dd className="m-0 break-words">{setting}{skip.optionType === 'config' && <> <code dir="ltr" className="font-mono text-muted">{visible(skip.option)}</code></>}</dd>
        <dt className="text-muted">{t('skip-approval-choice')}</dt>
        <dd className="m-0 break-words">{visible(skip.valueName)} <code dir="ltr" className="font-mono text-muted">{visible(skip.value)}</code></dd>
      </dl>
      <p className="m-0 text-xs text-muted">{t('skip-approval-scope')}</p>
      {!capable && <p role="note" className="m-0 text-xs text-needs-you">{t('skip-approval-other-window')}</p>}
      <div className="flex gap-2">
        <button type="button" className="button button--primary px-3 py-1 text-xs" disabled={busy || !answerable} onClick={() => onResolve?.(true)}>{t('approval-allow')}</button>
        <button type="button" className="button px-3 py-1 text-xs" disabled={busy || !answerable} onClick={() => onResolve?.(false)}>{t('approval-deny')}</button>
      </div>
    </article>
  );
}
