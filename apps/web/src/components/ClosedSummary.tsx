import { t } from '../i18n';
import { forgetReasonKey } from '../net/forget';
import { resumeReasonKey } from '../net/directory';
import type { AgentTileModel } from '../model';
import { ago, duration } from '../net/time';
import { Icon } from './Icon';
import { StateMark } from './StateBadge';

// What a closed agent leaves behind: how it ended, how long it ran, what it changed and cost. Its terminal is gone, so this is shown
// in its place, with the ways to look at the work and to clear it from the list.
export function ClosedSummary({ agent, cost, now, onReview, onHistory, onFiles, onDismiss, onForget, forgetError, onStart, onResume, resuming, resumeError }: {
  agent: AgentTileModel;
  cost?: string;
  now: number;
  onReview?: () => void;
  onHistory?: () => void;
  onFiles?: () => void;
  onDismiss: () => void;
  onForget?: () => void;
  // Why the hub did not forget it (an error code), shown instead of hiding the agent.
  forgetError?: string;
  onStart?: () => void;
  // ADR-0060, for an agent that was running when its machine stopped: Resume (its own session) or Start again (a new one there).
  onResume?: (newSession: boolean) => void;
  resuming?: boolean;
  resumeError?: string;
}) {
  // A non-zero exit code is also what stopping an agent leaves, so it is stated, not judged: the mark is the same either way.
  const code = agent.exitCode;
  const ran = agent.startedMs && agent.endedMs ? duration(agent.endedMs - agent.startedMs) : undefined;
  const changes = agent.filesChanged ?? 0;
  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-6">
      <div className="mx-auto grid max-w-xl gap-4">
        <div className="flex items-start gap-3">
          <span className="text-finished"><StateMark state="finished" /></span>
          <div className="min-w-0">
            <h2 className="m-0 text-base normal-case tracking-normal text-ink">{agent.name}</h2>
            <p className="m-0 text-finished">{code ? t('closed-failed', { code }) : t('closed-ok')}</p>
          </div>
        </div>
        <p className="muted m-0">{t(agent.interrupted ? (agent.interrupted.resumable ? 'interrupted-note' : 'interrupted-note-new') : 'closed-note')}</p>
        {agent.interrupted && onResume && <div className="flex flex-wrap gap-2">
          {agent.interrupted.resumable && <button type="button" className="button button--primary" disabled={resuming} onClick={() => onResume(false)}><Icon name="play" className="size-4" />{t('action-resume')}</button>}
          <button type="button" className={agent.interrupted.resumable ? 'button' : 'button button--primary'} disabled={resuming} onClick={() => onResume(true)}>{t('action-start-again')}</button>
        </div>}
        {resumeError && <p role="alert" className="m-0 text-needs-you">{t(resumeReasonKey(resumeError))}</p>}
        <dl className="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 rounded-m border border-line bg-surface-1 p-4 text-[13px]">
          <dt className="text-muted">{t('closed-machine')}</dt><dd className="m-0 truncate">{agent.machine}</dd>
          <dt className="text-muted">{t('closed-folder')}</dt><dd className="m-0 break-all font-mono text-xs">{agent.workspace || agent.project || '—'}</dd>
          {agent.branch && <><dt className="text-muted">{t('closed-branch')}</dt><dd className="m-0 font-mono text-xs">{agent.branch}</dd></>}
          <dt className="text-muted">{t('closed-ended-label')}</dt><dd className="m-0">{agent.endedMs ? ago(agent.endedMs, now) : t('closed-earlier')}</dd>
          {ran && <><dt className="text-muted">{t('closed-ran-label')}</dt><dd className="m-0">{ran}</dd></>}
          <dt className="text-muted">{t('closed-changes-label')}</dt><dd className="m-0">{changes > 0 ? t('changes-files', { count: changes }) : t('closed-changes-none')}</dd>
          {!!agent.tokens && <><dt className="text-muted">{t('closed-tokens-label')}</dt><dd className="m-0">{t('agent-tokens', { count: agent.tokens })}{cost ? ` · ${cost}` : ''}</dd></>}
        </dl>
        <div className="flex flex-wrap gap-2">
          {onReview && changes > 0 && <button type="button" className="button button--primary" onClick={onReview}>{t('closed-review')}</button>}
          {onHistory && <button type="button" className="button" onClick={onHistory}><Icon name="history" className="size-4" />{t('console-history')}</button>}
          {onFiles && <button type="button" className="button" onClick={onFiles}><Icon name="files" className="size-4" />{t('console-files')}</button>}
          {onStart && <button type="button" className="button" onClick={onStart}><Icon name="plus" className="size-4" />{t('action-start-agent')}</button>}
          <span className="flex-1" />
          <button type="button" className="button" onClick={onDismiss}>{t('closed-dismiss')}</button>
          {onForget && <button type="button" className="button" onClick={onForget}>{t('closed-forget')}</button>}
        </div>
        {forgetError && <p role="alert" className="m-0 text-needs-you">{t('closed-forget-failed', { reason: t(forgetReasonKey(forgetError)) })}</p>}
      </div>
    </div>
  );
}
