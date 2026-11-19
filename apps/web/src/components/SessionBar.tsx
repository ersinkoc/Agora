import { useEffect, useState } from 'react';
import { Icon } from './Icon';
import { Menu, type MenuItem } from '../layout/Menu';
import { duration } from '../net/time';
import { t } from '../i18n';
import type { SessionModel, SessionOptionModel } from '../model';
import { choices, confirmed, contextFill, contextLabel, contextPercent, isChange, needsConfirm, needsPlaceholder, NOT_REPORTED, ownCost, pendingFor, shortCount, type OptionKind, type PendingChoice } from './sessionChoice';

// ADR-0058: the mode and model a chat agent runs with, how full its context is, and what it says it cost. The pickers appear only
// for what the agent reported; a mode in which the agent acts without asking is chosen only after the user confirms it. Every choice
// names the agent (`agentKey`, hub-wide) it was made for, so a confirmation left open never reaches another agent.
export function SessionBar({ agentKey, session, choose, disabled, startedMs }: { agentKey: string; session: SessionModel; startedMs?: number; choose?: (agentKey: string, kind: OptionKind, id: string) => Promise<boolean>; disabled?: boolean }) {
  const [held, setPending] = useState<PendingChoice | undefined>();
  const pending = pendingFor(held, agentKey);
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  // How long it has run, when this window saw it start; moves once a second-ish (every 15 s is enough for minutes).
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { if (!startedMs || disabled) return; const id = setInterval(() => setNow(Date.now()), 15000); return () => clearInterval(id); }, [startedMs, disabled]);
  const elapsed = startedMs && !disabled ? Math.max(0, now - startedMs) : undefined;
  const pct = contextPercent(session);
  const label = contextLabel(session);
  const said = label ? t(label.key, { percent: label.percent }) : '';
  const cost = ownCost(session);
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
    if (option && needsConfirm(kind, option)) { setPending({ agentKey, option }); return; }
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
    const items: MenuItem[] = choices(list, current).map((o) => ({ label: name(o), checked: o.id === current, disabled: disabled || busy || !choose, run: () => pick(kind, list, o.id) }));
    return (
      <Menu label={`${word}: ${shown}`} items={items} align="start" className="ide-chip max-w-56 min-w-0 cursor-pointer hover:border-accent hover:text-ink">
        <span className="flex-none">{word}</span><span className="min-w-0 truncate font-medium text-ink">{shown}</span><Icon name="chevronDown" className="size-3" />
      </Menu>
    );
  };
  return (
    <div className="flex flex-none flex-col gap-1.5 border-b border-line pb-2">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5" role="group" aria-label={t('session-header')}>
        {picker('mode', session.modes, session.currentMode)}
        {picker('model', session.models, session.currentModel)}
        {pct !== undefined && (
          <span className="flex items-center gap-1.5 text-xs text-muted" title={t('session-context-detail', { used: shortCount(session.contextTokens), size: shortCount(session.contextWindow) })}>
            <span>{said}</span>
            {/* Drawn with design tokens, not a native <meter>: Chromium paints its empty track light grey, which reads as full in dark mode. */}
            <span role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-label={said}
              className="relative h-1.5 w-16 overflow-hidden rounded-full border border-line bg-surface-2">
              <span className={`absolute inset-y-0 start-0 rounded-full ${contextFill(pct)}`} style={{ width: `${session.contextTokens > 0 ? Math.max(pct, 2) : 0}%` }} />
            </span>
          </span>
        )}
        {elapsed !== undefined && <span className="ide-chip" title={t('session-elapsed-hint')}><Icon name="clock" className="size-3" />{duration(elapsed)}</span>}
        {cost !== undefined && <span className="ide-chip">{t('session-cost', { amount: cost })}</span>}
        {failed && <span role="status" className="text-xs text-needs-you">{t('session-change-failed')}</span>}
      </div>
      {pending && (
        <div role="alertdialog" aria-label={t('session-confirm-title')} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-s border border-needs-you/60 bg-surface-2 px-3 py-2">
          <div className="min-w-0 flex-1 basis-64 text-xs"><strong className="block">{t('session-confirm-title')}</strong>
            <span className="text-muted">{t('session-confirm-body', { name: pending.option.name || pending.option.id })}</span></div>
          <button type="button" className="button px-3 py-1 text-xs" onClick={() => { const c = confirmed(held, agentKey); setPending(undefined); if (c) void apply(c.agentKey, c.kind, c.id); }}>{t('session-confirm-yes')}</button>
          <button type="button" className="button button--primary px-3 py-1 text-xs" onClick={() => setPending(undefined)}>{t('session-confirm-no')}</button>
        </div>
      )}
    </div>
  );
}
