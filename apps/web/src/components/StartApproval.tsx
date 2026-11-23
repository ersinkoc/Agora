import { Fragment, useId, useState } from 'react';
import { t } from '../i18n';
import { approvalName } from '../net/approvalText';
import type { StartOption } from '../net/approval';
import { visible, type ApprovalModel, type StartRequest, type StartSetting } from '../net/view';
import { StateMark } from './StateBadge';
import { Button } from './kit';

export interface StartChoice { grant: number; nested: boolean }

// How many starts a "more like this" answer allows, this one included (the node takes at most 10).
export const GRANT_SIZE = 3;

// ADR-0066: a setting's name in the window's own words: the machine's label when it is one of the window's setting texts (`setting-model`,
// `setting-effort`), else the setting's id as text. Never the agent's words: the label comes from the machine's manifest, not the request.
// ADR-0059 decision 5: a launch option's name in the window's own words (`option-skip-permissions`, ...), else its id as text.
const optionName = (o: StartOption) => (/^option-[a-z0-9-]{1,32}$/.test(o.label) && t(o.label) !== o.label ? t(o.label) : visible(o.id));

const settingName = (st: StartSetting) => (/^setting-[a-z0-9-]{1,32}$/.test(st.label) && t(st.label) !== st.label ? t(st.label) : visible(st.id));

// ADR-0057: an agent asks to start another. Everything it asks for is shown before the user answers, all of it from the signed request
// itself (never from what the hub says around it): who asks and on which machine, what would start, where (marked when outside its own
// folder), whether in its own worktree, how deep, and the whole first prompt. It is the agent's text, shown as text only, with any
// hidden character written as a visible escape.
export function StartApproval({ approval, start, busy, onResolve }: {
  approval: ApprovalModel;
  start: StartRequest;
  busy?: boolean;
  onResolve: (approved: boolean, choice: StartChoice) => void;
}) {
  const [more, setMore] = useState(false);
  const [nested, setNested] = useState(false);
  // ADR-0059 decision 5: a helper that would act without asking is allowed only after the same confirmation a start from the window
  // asks for, with "Keep asking" as the primary button (which goes back to the card, where it can be denied).
  const skipping = start.options.filter((o) => o.skipsPermissions);
  const [confirming, setConfirming] = useState(false);
  // The same request can show in two places at once (its pane and the details panel): each has its own radio group.
  const group = useId();
  // The node gives a grant only when the request says it may (inside the asker's folder, not a shell); the choice is not offered otherwise.
  // The machine's and the agent's names as this window knows them; where it does not know one, words, never an id (the request's own
  // `node` and `agent` fields are ids).
  const who = visible(approvalName(approval));
  const title = approval.machineName?.trim()
    ? t('start-approval-title', { name: who, machine: visible(approval.machineName), kind: visible(start.kind) })
    : t('start-approval-title-nomachine', { name: who, kind: visible(start.kind) });
  const choice = (): StartChoice => (start.grantAllowed ? { grant: more ? GRANT_SIZE : 0, nested: nested && start.depth < 2 } : { grant: 0, nested: false });
  return (
    <article role="alert" aria-label={title}
      className="flex flex-col gap-2 border-b border-needs-you/60 bg-surface-2 px-3 py-2">
      <div className="flex items-center gap-2">
        <StateMark state="needs-you" />
        <strong className="min-w-0 flex-1 break-words">{title}</strong>
      </div>
      <dl className="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted">{t('start-approval-folder')}</dt>
        <dd className="m-0 break-all font-mono" dir="ltr">{visible(start.folder)}{start.worktree ? ` · ${t('start-approval-worktree')}` : ''}</dd>
        {start.settings.map((st, i) => <Fragment key={i}>
          <dt className="text-muted">{settingName(st)}</dt>
          <dd className="m-0 break-all font-mono" dir="ltr">{visible(st.value)}</dd>
        </Fragment>)}
        {start.options.length > 0 && <>
          <dt className="text-muted">{t('start-how-it-acts')}</dt>
          <dd className={`m-0 break-words ${skipping.length > 0 ? 'text-danger' : ''}`}>{start.options.map(optionName).join(', ')}</dd>
        </>}
      </dl>
      {skipping.length > 0 && <p className="m-0 text-xs text-danger">{t('start-approval-does-not-ask')}</p>}
      {start.depth > 1 && <p className="m-0 text-xs text-muted">{t('start-approval-helper')}</p>}
      {!start.inside && <p className="m-0 text-xs text-danger">{t('start-approval-outside')}</p>}
      <div className="text-xs">
        <span className="text-muted">{t('start-approval-prompt')}</span>
        {start.prompt
          ? <pre className="m-0 mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-xs border border-line bg-surface-0 p-2 font-mono text-xs">{visible(start.prompt)}</pre>
          : <p className="m-0 mt-1 text-muted">{t('start-approval-no-prompt')}</p>}
      </div>
      {start.inside && !start.grantAllowed && <p className="m-0 text-xs text-muted">{t(start.options.length > 0 ? 'start-approval-options-once' : start.settings.length > 0 ? 'start-approval-settings-once' : 'start-approval-no-grant')}</p>}
      {start.grantAllowed && <fieldset className="m-0 flex min-w-0 flex-col gap-1 border-0 p-0 text-xs">
        <legend className="sr-only">{t('start-approval-scope')}</legend>
        <label className="touch-row flex min-w-0 items-center gap-2"><input className="flex-none" type="radio" name={group} checked={!more} onChange={() => setMore(false)} /><span className="min-w-0 break-words">{t('start-approval-once')}</span></label>
        <label className="touch-row flex min-w-0 items-center gap-2"><input className="flex-none" type="radio" name={group} checked={more} onChange={() => setMore(true)} /><span className="min-w-0 break-words">{t('start-approval-more', { count: GRANT_SIZE, kind: visible(start.kind), name: who })}</span></label>
        {start.depth < 2 && <label className="touch-row flex min-w-0 items-center gap-2"><input className="flex-none" type="checkbox" checked={nested} onChange={(e) => setNested(e.target.checked)} /><span className="min-w-0 break-words">{t('start-approval-nested')}</span></label>}
      </fieldset>}
      {confirming && <div role="alertdialog" aria-label={t('session-confirm-title')} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-s border border-danger/60 bg-surface-2 px-3 py-2">
        <div className="min-w-0 flex-1 basis-64 text-xs"><strong className="block">{t('session-confirm-title')}</strong>
          <span className="text-muted">{t('start-confirm-skip-body', { name: skipping.map(optionName).join(', ') })}</span></div>
        <button type="button" className="button px-3 py-1 text-xs" disabled={busy} onClick={() => { setConfirming(false); onResolve(true, choice()); }}>{t('start-confirm-skip-yes')}</button>
        <button type="button" className="button button--primary px-3 py-1 text-xs" autoFocus onClick={() => setConfirming(false)}>{t('session-confirm-no')}</button>
      </div>}
      {/* Redesign PR 4: Deny, then Allow, as on every request. An Allow that starts a helper that would not ask is the danger button
          (ADR-0102), and it still opens the confirmation above first. */}
      <div className="flex gap-2">
        <Button size="sm" disabled={busy} onClick={() => onResolve(false, { grant: 0, nested: false })}>{t('approval-deny')}</Button>
        <Button size="sm" variant={skipping.length > 0 ? 'danger' : 'primary'} disabled={busy || confirming} onClick={() => (skipping.length > 0 ? setConfirming(true) : onResolve(true, choice()))}>{t('approval-allow')}</Button>
      </div>
    </article>
  );
}
