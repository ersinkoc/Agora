import { t } from '../i18n';
import type { ApprovalModel } from '../net/view';

export function ApprovalCard({ approval, busy, onResolve }: {
  approval: ApprovalModel;
  busy?: boolean;
  onResolve: (approved: boolean) => void;
}) {
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
