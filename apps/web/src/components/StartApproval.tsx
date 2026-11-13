import { useId, useState } from 'react';
import { t } from '../i18n';
import type { ApprovalModel, StartRequest } from '../net/view';
import { StateMark } from './StateBadge';

export interface StartChoice { grant: number; nested: boolean }

// How many starts a "more like this" answer allows, this one included (the node takes at most 10).
export const GRANT_SIZE = 3;

// ADR-0057: an agent asks to start another. Everything it asks for is shown before the user answers: who asks, what would start, where
// (marked when outside its own folder) and the whole first prompt. The prompt is the agent's text, shown as text only.
export function StartApproval({ approval, start, busy, onResolve }: {
  approval: ApprovalModel;
  start: StartRequest;
  busy?: boolean;
  onResolve: (approved: boolean, choice: StartChoice) => void;
}) {
  const [more, setMore] = useState(false);
  const [nested, setNested] = useState(false);
  // The same request can show in two places at once (its pane and the details panel): each has its own radio group.
  const group = useId();
  // The node gives a grant only when the request says it may (inside the asker's folder, not a shell); the choice is not offered otherwise.
  const choice = (): StartChoice => (start.grantAllowed ? { grant: more ? GRANT_SIZE : 0, nested: nested && start.depth < 2 } : { grant: 0, nested: false });
  return (
    <article role="alert" aria-label={t('start-approval-title', { name: approval.agentName, kind: start.kind })}
      className="flex flex-col gap-2 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
      <div className="flex items-center gap-2">
        <StateMark state="needs-you" />
        <strong className="min-w-0 flex-1">{t('start-approval-title', { name: approval.agentName, kind: start.kind })}</strong>
      </div>
      <dl className="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted">{t('start-approval-folder')}</dt>
        <dd className="m-0 break-all font-mono" dir="ltr">{start.folder}{start.worktree ? ` · ${t('start-approval-worktree')}` : ''}</dd>
      </dl>
      {!start.inside && <p className="m-0 text-xs text-needs-you">{t('start-approval-outside')}</p>}
      <div className="text-xs">
        <span className="text-muted">{t('start-approval-prompt')}</span>
        {start.prompt
          ? <pre className="m-0 mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-xs border border-line bg-surface-0 p-2 font-mono text-xs">{start.prompt}</pre>
          : <p className="m-0 mt-1 text-muted">{t('start-approval-no-prompt')}</p>}
      </div>
      {start.inside && !start.grantAllowed && <p className="m-0 text-xs text-muted">{t('start-approval-no-grant')}</p>}
      {start.grantAllowed && <fieldset className="m-0 flex flex-col gap-1 border-0 p-0 text-xs">
        <legend className="sr-only">{t('start-approval-scope')}</legend>
        <label className="flex items-center gap-2"><input type="radio" name={group} checked={!more} onChange={() => setMore(false)} />{t('start-approval-once')}</label>
        <label className="flex items-center gap-2"><input type="radio" name={group} checked={more} onChange={() => setMore(true)} />{t('start-approval-more', { count: GRANT_SIZE, kind: start.kind, name: approval.agentName })}</label>
        {start.depth < 2 && <label className="flex items-center gap-2"><input type="checkbox" checked={nested} onChange={(e) => setNested(e.target.checked)} />{t('start-approval-nested')}</label>}
      </fieldset>}
      <div className="flex gap-2">
        <button type="button" className="button button--primary px-3 py-1 text-xs" disabled={busy} onClick={() => onResolve(true, choice())}>{t('approval-allow')}</button>
        <button type="button" className="button px-3 py-1 text-xs" disabled={busy} onClick={() => onResolve(false, { grant: 0, nested: false })}>{t('approval-deny')}</button>
      </div>
    </article>
  );
}
