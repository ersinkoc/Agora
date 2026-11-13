import { useState } from 'react';
import { t } from '../i18n';
import type { SessionModel, SessionOptionModel } from '../model';
import { choices, contextPercent, needsConfirm, ownCost, shortCount, type OptionKind } from './sessionChoice';

// ADR-0058: the mode and model a chat agent runs with, how full its context is, and what it says it cost. The pickers appear only
// for what the agent reported; a mode in which the agent acts without asking is chosen only after the user confirms it.
export function SessionBar({ session, choose, disabled }: { session: SessionModel; choose?: (kind: OptionKind, id: string) => Promise<boolean>; disabled?: boolean }) {
  const [pending, setPending] = useState<SessionOptionModel | undefined>();
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const pct = contextPercent(session);
  const cost = ownCost(session);
  const apply = async (kind: OptionKind, id: string) => {
    if (!choose) return;
    setBusy(true);
    const ok = await choose(kind, id);
    setBusy(false);
    setFailed(!ok);
  };
  const pick = (kind: OptionKind, list: readonly SessionOptionModel[], id: string) => {
    const option = list.find((o) => o.id === id);
    setFailed(false);
    if (needsConfirm(kind, option)) { setPending(option); return; }
    setPending(undefined);
    void apply(kind, id);
  };
  const picker = (kind: OptionKind, list: readonly SessionOptionModel[], current: string) => list.length > 0 && (
    <label className="flex min-w-0 items-center gap-1.5 text-xs text-muted">
      <span>{t(kind === 'mode' ? 'session-mode' : 'session-model')}</span>
      <select className="min-w-0 max-w-48 truncate rounded-xs border border-line bg-surface-1 px-1.5 py-0.5 text-xs text-ink"
        value={current} disabled={disabled || busy || !choose} onChange={(e) => pick(kind, list, e.target.value)}>
        {choices(list, current).map((o) => <option key={o.id} value={o.id} title={o.description || undefined}>
          {o.skipsPermissions ? t('session-mode-unasked', { name: o.name || o.id }) : (o.name || o.id)}
        </option>)}
      </select>
    </label>
  );
  return (
    <div className="flex flex-none flex-col gap-1.5 border-b border-line pb-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        {picker('mode', session.modes, session.currentMode)}
        {picker('model', session.models, session.currentModel)}
        {pct !== undefined && (
          <span className="flex items-center gap-1.5 text-xs text-muted" title={t('session-context-detail', { used: shortCount(session.contextTokens), size: shortCount(session.contextWindow) })}>
            <span>{t('session-context', { percent: pct })}</span>
            <meter className="h-1.5 w-16" min={0} max={100} low={70} high={90} optimum={0} value={pct} aria-label={t('session-context', { percent: pct })} />
          </span>
        )}
        {cost !== undefined && <span className="ide-chip">{t('session-cost', { amount: cost })}</span>}
        {failed && <span role="status" className="text-xs text-needs-you">{t('session-change-failed')}</span>}
      </div>
      {pending && (
        <div role="alertdialog" aria-label={t('session-confirm-title')} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-s border border-needs-you/60 bg-surface-2 px-3 py-2">
          <div className="min-w-0 flex-1 basis-64 text-xs"><strong className="block">{t('session-confirm-title')}</strong>
            <span className="text-muted">{t('session-confirm-body', { name: pending.name || pending.id })}</span></div>
          <button type="button" className="button px-3 py-1 text-xs" onClick={() => { const id = pending.id; setPending(undefined); void apply('mode', id); }}>{t('session-confirm-yes')}</button>
          <button type="button" className="button button--primary px-3 py-1 text-xs" onClick={() => setPending(undefined)}>{t('session-confirm-no')}</button>
        </div>
      )}
    </div>
  );
}
