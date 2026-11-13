import { t } from '../i18n';
import type { ApprovalModel } from '../net/view';
import { StartApproval, type StartChoice } from './StartApproval';

export function ApprovalCard({ approval, busy, onResolve }: {
  approval: ApprovalModel;
  busy?: boolean;
  onResolve: (approved: boolean, choice?: StartChoice) => void;
}) {
  // ADR-0057: a request to start an agent shows everything it asks for.
  if (approval.start) return <StartApproval approval={approval} start={approval.start} busy={busy} onResolve={onResolve} />;
  return (
    <article className="approval" aria-label={t('approval-title', { name: approval.agentName })}>
      <strong>{t('approval-title', { name: approval.agentName })}</strong>
      <span>{approval.action}</span>
      {approval.detail && <span className="muted">{approval.detail}</span>}
      <div className="approval__actions">
        <button type="button" className="button button--primary" disabled={busy} onClick={() => onResolve(true)}>{t('approval-allow')}</button>
        <button type="button" className="button" disabled={busy} onClick={() => onResolve(false)}>{t('approval-deny')}</button>
      </div>
    </article>
  );
}
