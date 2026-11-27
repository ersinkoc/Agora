import { createContext, useContext, useState } from 'react';
import { agora } from '../gen/agora.js';
import { t } from '../i18n';
import { visible, type TaskReviewModel } from '../net/view';

type Read = agora.v1.TaskReviewRead.$Properties;
type Decide = agora.v1.TaskReviewDecide.$Properties;
type Report = agora.v1.TaskReviewReport.$Properties;
type Decision = agora.v1.TaskReviewDecision;

export const BoardReviewContext = createContext<{
  read: (request: Read) => Promise<Report>;
  decide?: (request: Decide) => Promise<void>;
} | undefined>(undefined);

export function BoardReview({ projectId, taskId, review }: { projectId: string; taskId: string; review: TaskReviewModel }) {
  const api = useContext(BoardReviewContext);
  const [report, setReport] = useState<Report>();
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<'read' | 'decision'>();
  const [submitted, setSubmitted] = useState(false);
  if (!api) return null;
  const request = { projectId, taskId, reportRevision: review.revision };

  async function read() {
    if (!api || busy) return;
    setBusy(true);
    setProblem(undefined);
    try { setReport(await api.read(request)); }
    catch { setProblem('read'); }
    finally { setBusy(false); }
  }

  async function decide(decision: Decision) {
    if (!api?.decide || !report || busy || submitted || review.state !== 'pending') return;
    setBusy(true);
    setProblem(undefined);
    try {
      await api.decide({ ...request, decision });
      setSubmitted(true);
    } catch { setProblem('decision'); }
    finally { setBusy(false); }
  }

  const decisions = agora.v1.TaskReviewDecision;
  return <div className="grid min-w-0 gap-1.5 border-t border-line pt-1.5 text-sm">
    {!report && <button type="button" className="kit-btn kit-btn--ghost min-h-11 w-fit" disabled={busy}
      onClick={() => void read()}>{t('board-review-read')}</button>}
    {report && <>
      <p className="m-0 whitespace-pre-wrap [overflow-wrap:anywhere]" dir="auto">{visible(report.summary ?? '')}</p>
      {!!report.refs?.length && <ul className="m-0 grid gap-1 break-all ps-4 text-xs text-muted">
        {report.refs.map((ref, i) => <li key={i} dir="auto">{visible(ref)}</li>)}
      </ul>}
      {api.decide && review.state === 'pending' && <div className="flex flex-wrap gap-1">
        <button type="button" className="kit-btn kit-btn--ghost min-h-11" disabled={busy || submitted}
          onClick={() => void decide(decisions.TASK_REVIEW_DECISION_RETURN)}>{t('board-review-return')}</button>
        <button type="button" className="kit-btn kit-btn--primary min-h-11" disabled={busy || submitted}
          onClick={() => void decide(decisions.TASK_REVIEW_DECISION_ACCEPT)}>{t('board-review-accept')}</button>
      </div>}
    </>}
    {problem && <span role="alert">{t(`board-review-${problem}-failed`)}</span>}
  </div>;
}
