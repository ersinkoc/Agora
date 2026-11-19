import { useEffect, useMemo, useState } from 'react';
import { settingName } from './launchSettings';
import { startedWith } from './launchMemory';
import { Icon } from './Icon';
import { Menu, type MenuItem } from '../layout/Menu';
import { duration } from '../net/time';
import { t } from '../i18n';
import type { SessionModel, SessionOptionModel, UiState } from '../model';
import { choices, confirmed, contextFill, contextLabel, contextPercent, isChange, needsConfirm, needsPlaceholder, NOT_REPORTED, pendingFor, shortCount, type OptionKind, type PendingChoice } from './sessionChoice';

// ADR-0058: the mode and model a chat agent runs with, how full its context is, and what it says it cost. The pickers appear only
// for what the agent reported; a mode in which the agent acts without asking is chosen only after the user confirms it. Every choice
// names the agent (`agentKey`, hub-wide) it was made for, so a confirmation left open never reaches another agent.
// `state` and `tokens` belong to the header (`status`): the agent's state as a pill with its running time, and the tokens used so far.
const PILL: Record<UiState, string> = {
  working: 'border-working/40 text-working', 'needs-you': 'border-needs-you/50 text-needs-you', finished: 'border-finished/40 text-finished', idle: 'border-line text-muted', unknown: 'border-line text-muted',
};
export function SessionBar({ agentKey, session, choose, disabled, startedMs, state, tokens, part = 'all' }: { part?: 'all' | 'controls' | 'status'; agentKey: string; session: SessionModel; startedMs?: number; state?: UiState; tokens?: number; choose?: (agentKey: string, kind: OptionKind, id: string) => Promise<boolean>; disabled?: boolean }) {
  const [held, setPending] = useState<PendingChoice | undefined>();
  const pending = pendingFor(held, agentKey);
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  // How long it has run, when this window saw it start; moves once a second-ish (every 15 s is enough for minutes).
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { if (!startedMs || disabled) return; const id = setInterval(() => setNow(Date.now()), 30000); return () => clearInterval(id); }, [startedMs, disabled]);
  const elapsed = startedMs && !disabled ? Math.max(0, now - startedMs) : undefined;
  const pct = contextPercent(session);
  const label = contextLabel(session);
  const said = label ? t(label.key, { percent: label.percent }) : '';
  const apply = async (target: string, kind: OptionKind, id: string) => {
    if (!choose) return;
    setBusy(true);
    const ok = await choose(target, kind, id);
    setBusy(false);
    setFailed(!ok);
  };
  const pick = (kind: OptionKind, list: readonly SessionOptionModel[], id: string) => {
    if (!isChange(kind === 'mode' ? session.currentMode : session.currentModel, id)) return; // the placeholder and the one in use are not choices
    const option = list.find((o) => o.id === id);
    setFailed(false);
    // Never reached from the menu (such a choice is disabled there), kept as a guard: the node refuses a switch to a mode that skips
    // permission questions until a signed confirmation exists, so no dialog is offered for one that can never succeed.
    if (option && needsConfirm(kind, option)) return;
    setPending(undefined);
    void apply(agentKey, kind, id);
  };
  // A custom menu, not a native select: it looks the same on every platform and in both themes, and lists only what the agent reported.
  const picker = (kind: OptionKind, list: readonly SessionOptionModel[], current: string) => {
    if (list.length === 0) return null;
    const word = t(kind === 'mode' ? 'session-mode' : 'session-model');
    const name = (o: SessionOptionModel) => (o.skipsPermissions ? t('session-mode-unasked', { name: o.name || o.id }) : (o.name || o.id));
    const now = list.find((o) => o.id === current);
    const shown = now ? name(now) : needsPlaceholder(current) ? t('session-not-reported') : current;
    // A mode that skips permission questions cannot be switched to from here yet (the node refuses it): shown, disabled, with why.
    const items: MenuItem[] = choices(list, current).map((o) => { const unavailable = needsConfirm(kind, o) && o.id !== current; return { label: name(o), checked: o.id === current, disabled: disabled || busy || !choose || unavailable, hint: unavailable ? t('session-unasked-unavailable-hint') : undefined, run: () => pick(kind, list, o.id) }; });
    return (
      <Menu label={`${word}: ${shown}`} items={items} align="start" className="ide-chip h-7 max-w-56 min-w-0 cursor-pointer rounded-s px-2.5 hover:border-accent hover:text-ink">
        <span className="flex-none">{word}</span><span className="min-w-0 truncate font-medium text-ink">{shown}</span><Icon name="chevronDown" className="size-3" />
      </Menu>
    );
  };
  // What this device chose when it started the agent (an effort, say): the agent does not report it, so it is shown read-only and only
  // when known. A model or mode already has its own menu.
  const started = useMemo(() => startedWith(agentKey).filter(([id]) => !(id === 'model' && session.models.length > 0) && !(id === 'mode' && session.modes.length > 0)), [agentKey, session.models.length, session.modes.length]);
  const controls = <>
    {picker('mode', session.modes, session.currentMode)}
    {picker('model', session.models, session.currentModel)}
    {started.map(([id, value]) => <span key={id} className="ide-chip h-7 rounded-s px-2.5" title={t('session-started-hint')}>{t('session-started-with', { name: settingName({ id, label: `setting-${id}`, values: [], defaultValue: '' }, t), value })}</span>)}
    {session.modes.some((o) => needsConfirm('mode', o) && o.id !== session.currentMode) && <span role="note" className="text-xs text-muted">{t('session-unasked-unavailable')}</span>}
    {failed && <span role="status" className="text-xs text-needs-you">{t('session-change-failed')}</span>}
  </>;
  const word = state ? t(`state-${state}`) : '';
  const status = <>
    {state && (
      <span className={`inline-flex h-6 items-center gap-2 rounded-full border bg-surface-2 px-2.5 text-xs whitespace-nowrap ${PILL[state]}`} title={elapsed !== undefined ? t('session-elapsed-hint') : undefined} data-state-pill={state}>
        <span aria-hidden className={`chat-dot size-2 rounded-full bg-current ${state === 'working' ? 'chat-dot--pulse' : ''}`} />
        {elapsed !== undefined && elapsed >= 60000 ? t('chat-state-since', { state: word, time: elapsed < 3_600_000 ? `${Math.floor(elapsed / 60000)} min` : duration(Math.floor(elapsed / 60000) * 60000) }) : word}
      </span>
    )}
    {!state && elapsed !== undefined && <span className="ide-chip" title={t('session-elapsed-hint')}><Icon name="clock" className="size-3" />{duration(elapsed)}</span>}
    {state && <span className="flex-1" />}
    {pct !== undefined && (
      <span className="flex items-center gap-2 text-xs text-muted" title={t('session-context-detail', { used: shortCount(session.contextTokens), size: shortCount(session.contextWindow) })}>
        <span>{said}</span>
        {/* Drawn with design tokens, not a native <meter>: Chromium paints its empty track light grey, which reads as full in dark mode. */}
        <span role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-label={said}
          className="relative h-1.5 w-16 overflow-hidden rounded-full border border-line bg-surface-2">
          <span className={`absolute inset-y-0 start-0 rounded-full ${contextFill(pct)}`} style={{ width: `${session.contextTokens > 0 ? Math.max(pct, 2) : 0}%` }} />
        </span>
      </span>
    )}
    {tokens ? <span className="text-xs text-muted tabular-nums">{t('usage-total-tokens', { count: tokens, short: shortCount(tokens) })}</span> : null}
  </>;
  const confirm = pending && (
    <div role="alertdialog" aria-label={t('session-confirm-title')} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-s border border-needs-you/60 bg-surface-2 px-3 py-2">
      <div className="min-w-0 flex-1 basis-64 text-xs"><strong className="block">{t('session-confirm-title')}</strong>
        <span className="text-muted">{t('session-confirm-body', { name: pending.option.name || pending.option.id })}</span></div>
      <button type="button" className="button px-3 py-1 text-xs" onClick={() => { const c = confirmed(held, agentKey); setPending(undefined); if (c) void apply(c.agentKey, c.kind, c.id); }}>{t('session-confirm-yes')}</button>
      <button type="button" className="button button--primary px-3 py-1 text-xs" onClick={() => setPending(undefined)}>{t('session-confirm-no')}</button>
    </div>
  );
  // The controls (mode, model) belong with the composer and the status (context, time) with the header; `all` keeps them in one row.
  if (part === 'controls') return <div className="flex min-w-0 flex-col gap-1.5"><div className="flex flex-wrap items-center gap-x-2 gap-y-1.5" role="group" aria-label={t('session-header')}>{controls}</div>{confirm}</div>;
  if (part === 'status') return pct === undefined && elapsed === undefined && !state && !tokens ? null : <div className="flex w-full flex-wrap items-center gap-x-3 gap-y-1.5">{status}</div>;
  return (
    <div className="flex flex-none flex-col gap-1.5 border-b border-line pb-2">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5" role="group" aria-label={t('session-header')}>{controls}{status}</div>
      {confirm}
    </div>
  );
}
