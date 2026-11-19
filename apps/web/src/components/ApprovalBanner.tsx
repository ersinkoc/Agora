import { t } from '../i18n';
import { approvalName } from '../net/approvalText';
import { detailText } from '../net/tool';
import type { ApprovalModel } from '../net/view';
import { StateMark } from './StateBadge';
import { FullRequest } from './ApprovalCard';
import { StartApproval, type StartChoice } from './StartApproval';

// What one agent waits on, with Allow and Deny: the same row in the window and in the full-size view.
// `pinned`: drawn as a card above a chat composer (rounded, tinted, with a shadow) instead of a full-width bar under the pane's header.
export function ApprovalBanner({ approval: x, onResolve, pinned }: { approval: ApprovalModel; onResolve?: (approval: ApprovalModel, approved: boolean, choice?: StartChoice) => void; pinned?: boolean }) {
  // ADR-0057: a request to start an agent shows everything it asks for (read-only where it cannot be answered).
  if (x.start) return <StartApproval approval={x} start={x.start} busy={!onResolve} onResolve={(ok, choice) => onResolve?.(x, ok, choice)} />;
  return (
    <div role="alert" className={pinned ? 'flex flex-wrap items-center gap-x-3 gap-y-2 rounded-m border border-needs-you/55 bg-needs-you/8 px-3.5 py-2.5 shadow-card' : 'flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-needs-you/60 bg-surface-2 px-3 py-2'}>
      <StateMark state="needs-you" />
      <div className="min-w-0 flex-1"><strong className={`block ${pinned ? 'line-clamp-2' : 'truncate'}`}>{t('approval-title', { name: approvalName(x) })}</strong><span className={`block text-xs text-muted ${pinned ? 'line-clamp-2' : 'truncate'}`} title={x.detail || undefined}>{x.action}{x.detail ? ` · ${detailText(x.detail)}` : ''}</span>{x.detail && <FullRequest detail={x.detail} />}</div>
      {onResolve && <>
        <button type="button" className="button button--primary px-3 py-1 text-xs" onClick={() => onResolve(x, true)}>{t('approval-allow')}</button>
        <button type="button" className="button px-3 py-1 text-xs" onClick={() => onResolve(x, false)}>{t('approval-deny')}</button>
      </>}
    </div>
  );
}
