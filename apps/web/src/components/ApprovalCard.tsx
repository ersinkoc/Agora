import { t } from '../i18n';
import { detailText } from '../net/tool';
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
      {approval.detail && <span className="muted">{detailText(approval.detail)}</span>}
      {/* The short form names the command or file; what the agent asked for, in full, stays one click away. */}
      {approval.detail && detailText(approval.detail) !== approval.detail && (
        <details className="text-xs text-muted">
          <summary className="cursor-pointer">{t('approval-full-request')}</summary>
          <pre dir="ltr" className="m-0 mt-1 max-h-48 overflow-auto font-mono break-all whitespace-pre-wrap">{approval.detail}</pre>
        </details>
      )}
      <div className="approval__actions">
        <button type="button" className="button button--primary" disabled={busy} onClick={() => onResolve(true)}>{t('approval-allow')}</button>
        <button type="button" className="button" disabled={busy} onClick={() => onResolve(false)}>{t('approval-deny')}</button>
      </div>
    </article>
  );
}
