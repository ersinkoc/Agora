import { useId, useState, type ReactElement, type ReactNode } from 'react';
import { t } from '../i18n';
import { approvalName } from '../net/approvalText';
import type { StartOption } from '../net/approval';
import { visible, type ApprovalModel, type StartRequest, type StartSetting } from '../net/view';
import { ApprovalRequest, ASK_ICON, type ApprovalDensity } from './ApprovalRequest';
import { Button, KeyValue, SegmentedControl } from './kit';

export interface StartChoice { grant: number; nested: boolean }

// How many starts a "more like this" answer allows, this one included (the node takes at most 10).
export const GRANT_SIZE = 3;
// ADR-0119 decision 12: for a start another machine asked for, how many of its helpers a standing grant lets run here at once (at most 20).
export const REMOTE_GRANT_SIZE = 6;

// ADR-0066: a setting's name in the window's own words: the machine's label when it is one of the window's setting texts (`setting-model`,
// `setting-effort`), else the setting's id as text. Never the agent's words: the label comes from the machine's manifest, not the request.
// ADR-0059 decision 5: a launch option's name in the window's own words (`option-skip-permissions`, ...), else its id as text.
export const optionName = (o: StartOption) => (/^option-[a-z0-9-]{1,32}$/.test(o.label) && t(o.label) !== o.label ? t(o.label) : visible(o.id));

const settingName = (st: StartSetting) => (/^setting-[a-z0-9-]{1,32}$/.test(st.label) && t(st.label) !== st.label ? t(st.label) : visible(st.id));

/** The title of a start request: who asks (on which machine, unless `machine` is false: the card shows it beside the title) and what
 *  kind it would start. The machine's and the agent's names as this window knows them; where it does not know one, words, never an id
 *  (the request's own `node` and `agent` fields are ids). */
export function startTitle(approval: ApprovalModel, start: StartRequest, machine = true): string {
  // ADR-0119: another machine's agent asks this one; the asking machine is the hub's word, the agent's kind that machine's.
  if (start.remote) {
    return t('start-approval-remote-title', { agentKind: visible(start.remote.originAgentKind || start.remote.originAgent), from: visible(start.remote.originNode), machine: visible(approval.machineName?.trim() || start.node), kind: visible(start.kind) });
  }
  const who = visible(approvalName(approval));
  return machine && approval.machineName?.trim()
    ? t('start-approval-title', { name: who, machine: visible(approval.machineName), kind: visible(start.kind) })
    : t('start-approval-title-nomachine', { name: who, kind: visible(start.kind) });
}

// Redesign PR 5: a start request in short, for a card that offers Review (never Allow): two lines at most, each cut at the end of the
// line. The first says how and where (the kind is in the title): whether it would act without asking or run outside the asker's folder
// (first, so they are never the part that is cut), then the folder; the second quotes the first line of the prompt and says how many
// lines follow.
// The flags are never cut (only flags too wide for the card wrap). All of it is the agent's text, shown as text with any hidden
// character written as a visible escape. Nothing is decided on it: the whole request is read in Review (`StartApproval`, full).
export function StartSummary({ start }: { start: StartRequest }) {
  const skipping = start.options.some((o) => o.skipsPermissions);
  const lines = visible(start.prompt).split(/\r?\n/);
  const at = lines.findIndex((l) => l.trim() !== '');
  const rest = at < 0 ? 0 : lines.slice(at + 1).filter((l) => l.trim() !== '').length;
  return (
    <div className="approval-req__summary" data-start-summary>
      <p className="approval-req__line approval-req__line--facts">
        {start.options.length > 0 && <span className={skipping ? 'text-danger' : undefined}>{start.options.map(optionName).join(', ')}</span>}
        {!start.inside && <span className="text-danger">{t('start-summary-outside')}</span>}
        <span className="approval-req__clip font-mono" dir="ltr">{visible(start.folder)}{start.worktree ? ` · ${t('start-approval-worktree')}` : ''}</span>
      </p>
      <p className="approval-req__line">
        {at < 0 ? <span className="approval-req__clip">{t('start-approval-no-prompt')}</span>
          : <span className="approval-req__clip approval-req__quote" dir="auto">{lines[at]!.trim()}</span>}
        {rest > 0 && <span>{t('start-summary-more', { count: rest })}</span>}
      </p>
    </div>
  );
}

// ADR-0057: an agent asks to start another. Everything it asks for is shown before the user answers, all of it from the signed request
// itself (never from what the hub says around it): who asks and on which machine, what would start, where (marked when outside its own
// folder), whether in its own worktree, how deep, and the whole first prompt. It is the agent's text, shown as text only, with any
// hidden character written as a visible escape. Redesign PR 5: this is the full request, in the request frame: Review's dialog
// (`full`, the prompt whole in the dialog's own scroll) and the phone's card. Panes and the side panel show `StartSummary` with Review.
// Without `onResolve` (a viewer's window, ADR-0106) it is read-only: the whole request, with no answers and no choice of a grant.
export function StartApproval({ approval, start, busy, onResolve, density = 'card', titleWrap, lead, foot }: {
  approval: ApprovalModel;
  start: StartRequest;
  busy?: boolean;
  onResolve?: (approved: boolean, choice: StartChoice) => void;
  density?: ApprovalDensity;
  titleWrap?: (title: ReactElement) => ReactNode;
  // Before the answers (Review's Close).
  lead?: ReactNode;
  foot?: ReactNode;
}) {
  const [more, setMore] = useState(false);
  const [nested, setNested] = useState(false);
  // ADR-0059 decision 5: a helper that would act without asking is allowed only after the same confirmation a start from the window
  // asks for, with "Keep asking" as the primary button (which goes back to the card, where it can be denied).
  const skipping = start.options.filter((o) => o.skipsPermissions);
  const [confirming, setConfirming] = useState(false);
  // The same request can show in two places at once (its pane and the details panel): each names its own choice.
  const scope = useId();
  // The node gives a grant only when the request says it may (inside the asker's folder, not a shell); the choice is not offered otherwise.
  const who = visible(approvalName(approval));
  const title = startTitle(approval, start);
  const size = start.remote ? REMOTE_GRANT_SIZE : GRANT_SIZE;
  const choice = (): StartChoice => (start.grantAllowed ? { grant: more ? size : 0, nested: nested && start.depth < 2 } : { grant: 0, nested: false });
  const moreText = start.remote
    ? t('start-approval-remote-more', { count: size, kind: visible(start.kind), from: visible(start.remote.originNode) })
    : t('start-approval-more', { count: size, kind: visible(start.kind), name: who });
  const facts = [
    { label: t('start-approval-folder'), value: <span dir="ltr">{visible(start.folder)}{start.worktree ? ` · ${t('start-approval-worktree')}` : ''}</span>, mono: true },
    ...start.settings.map((st) => ({ label: settingName(st), value: <span dir="ltr">{visible(st.value)}</span>, mono: true })),
    ...(start.options.length > 0 ? [{ label: t('start-how-it-acts'), value: <span className={skipping.length > 0 ? 'text-danger' : ''}>{start.options.map(optionName).join(', ')}</span> }] : []),
  ];
  return (
    <ApprovalRequest density={density} alert={density !== 'full'} icon={ASK_ICON.start} title={title} titleWrap={titleWrap} className="approval-req--start" foot={foot}
      actions={<>
        {lead}
        {/* Redesign PR 4: Deny, then Allow, as on every request. An Allow that starts a helper that would not ask is the danger button
            (ADR-0102), and it still opens the confirmation below first. */}
        {onResolve && <>
          <Button size="sm" disabled={busy} onClick={() => onResolve(false, { grant: 0, nested: false })}>{t('approval-deny')}</Button>
          <Button size="sm" variant={skipping.length > 0 ? 'danger' : 'primary'} disabled={busy || confirming} onClick={() => (skipping.length > 0 ? setConfirming(true) : onResolve(true, choice()))}>{t('approval-allow')}</Button>
        </>}
      </>}>
      <div className="flex flex-col gap-2 text-xs">
        <KeyValue items={facts} />
        {skipping.length > 0 && <p className="m-0 text-danger">{t('start-approval-does-not-ask')}</p>}
        {start.remote && <p className="m-0 text-muted" data-remote-start>{t('start-approval-remote-note', { from: visible(start.remote.originNode) })}</p>}
        {start.depth > (start.remote ? 0 : 1) && <p className="m-0 text-muted">{t('start-approval-helper')}</p>}
        {!start.inside && <p className="m-0 text-danger">{t('start-approval-outside')}</p>}
        <div>
          <span className="text-muted">{t('start-approval-prompt')}</span>
          {start.prompt
            ? <pre className={`m-0 mt-1 whitespace-pre-wrap break-words rounded-xs border border-line bg-surface-0 p-2 font-mono text-xs${density === 'full' ? '' : ' max-h-48 overflow-auto'}`}>{visible(start.prompt)}</pre>
            : <p className="m-0 mt-1 text-muted">{t('start-approval-no-prompt')}</p>}
        </div>
        {start.inside && !start.grantAllowed && <p className="m-0 text-muted">{t(start.options.length > 0 ? 'start-approval-options-once' : start.settings.length > 0 ? 'start-approval-settings-once' : 'start-approval-no-grant')}</p>}
        {/* #1070: the grant is the kit's segmented control (one tab stop, the arrow keys choose). Its segments are short so they are never
            cut; what "More like this" allows is said in full under it, always (it never appears or goes with the choice, so nothing under
            it moves while the person chooses). What is sent is unchanged: `choice()`. */}
        {onResolve && start.grantAllowed && <div className="flex min-w-0 flex-col items-start gap-1.5" data-grant-choice>
          <span id={`${scope}-label`} className="text-muted">{t('start-approval-scope')}</span>
          <SegmentedControl labelledBy={`${scope}-label`} value={more ? 'more' : 'once'} onChange={(v) => setMore(v === 'more')}
            items={[{ value: 'once', label: t('start-approval-once') }, { value: 'more', label: t('start-approval-more-short'), title: moreText }]} />
          <p className="m-0 min-w-0 break-words text-muted" data-grant-more>{moreText}</p>
          {start.depth < 2 && <label className="touch-row flex min-w-0 items-center gap-2"><input className="flex-none" type="checkbox" checked={nested} onChange={(e) => setNested(e.target.checked)} /><span className="min-w-0 break-words">{t(start.remote ? 'start-approval-remote-nested' : 'start-approval-nested')}</span></label>}
        </div>}
        {onResolve && confirming && <div role="alertdialog" aria-label={t('session-confirm-title')} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-s border border-danger/60 bg-surface-2 px-3 py-2">
          <div className="min-w-0 flex-1 basis-64"><strong className="block">{t('session-confirm-title')}</strong>
            <span className="text-muted">{t('start-confirm-skip-body', { name: skipping.map(optionName).join(', ') })}</span></div>
          <button type="button" className="button px-3 py-1 text-xs" disabled={busy} onClick={() => { setConfirming(false); onResolve(true, choice()); }}>{t('start-confirm-skip-yes')}</button>
          <button type="button" className="button button--primary px-3 py-1 text-xs" autoFocus onClick={() => setConfirming(false)}>{t('session-confirm-no')}</button>
        </div>}
      </div>
    </ApprovalRequest>
  );
}
