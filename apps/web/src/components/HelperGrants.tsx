import { useCallback, useEffect, useRef, useState } from 'react';
import { t } from '../i18n';

// A live "more helpers like this" permission of one agent (ADR-0057), as its machine lists it.
export interface HelperGrant {
  id: string; kind: string; folder: string; remaining: number;
  /** ADR-0119: a standing grant another machine gave (and keeps): that machine, its name, and how many may run at once there. */
  machine?: string; machineName?: string; max?: number;
  /** ADR-0119: listed on the machine that keeps it: who it lets ask (`<agent> on <machine>`). */
  from?: string;
}

// ADR-0057: what an agent may still start without asking, with a way to take it back. Shown for an agent that has helpers.
export function HelperGrants({ view, list, revoke, refresh }: {
  /** Whose permissions these are (the agent's id): the view a revoke belongs to. The functions may be new on every render. */
  view: string;
  list: () => Promise<HelperGrant[]>;
  revoke: (grant: HelperGrant) => Promise<void>;
  refresh: number; // changes when the agent's helpers change, so the remaining count is read again
}) {
  const [grants, setGrants] = useState<HelperGrant[]>([]);
  const [failed, setFailed] = useState(false);
  const gen = useRef(0);
  // The newest functions, read when used: a parent that passes new wrappers on every render neither reloads the list nor ends the view.
  const listNow = useRef(list);
  listNow.current = list;
  const load = useCallback(() => {
    const current = ++gen.current;
    listNow.current()
      .then((g) => { if (gen.current === current) { setGrants(g); setFailed(false); } })
      .catch(() => { if (gen.current === current) setFailed(true); });
  }, []);
  useEffect(() => {
    load();
    return () => { gen.current += 1; };
  }, [load, view, refresh]);
  // A revoke belongs to the view it was made in (this agent), not to the list read that was current when it was clicked: every
  // revoke that succeeds while the view lasts reads the list again, even when another revoke's read came in between.
  const scope = useRef(0);
  useEffect(() => () => { scope.current += 1; }, [view]);
  if (grants.length === 0 && !failed) return null;
  return <div className="flex flex-col gap-1 border-b border-line bg-surface-1 px-3 py-2 text-xs" aria-label={t('helper-grants')}>
    {grants.map((g) => <div key={`${g.machine ?? ''}/${g.id}`} className="flex flex-wrap items-center gap-2">
      <span className="min-w-0 flex-1" {...(g.machine ? { 'data-remote-grant': '' } : {})}>{g.from
        ? t('helper-grant-held-line', { from: g.from, count: g.max ?? 0, kind: g.kind || t('helper-grant-any-kind'), n: g.remaining })
        : g.machine
          ? t('helper-grant-remote-line', { count: g.max ?? 0, kind: g.kind || t('helper-grant-any-kind'), machine: g.machineName || g.machine, n: g.remaining })
          : t('helper-grant-line', { count: g.remaining, kind: g.kind })} <code dir="ltr" className="break-all text-muted">{g.folder}</code></span>
      <button type="button" className="button px-2 py-1 text-xs" onClick={() => {
        const view = scope.current;
        void revoke(g).then(
          () => { if (scope.current === view) load(); },
          () => { if (scope.current === view) setFailed(true); },
        );
      }}>{t('helper-grant-revoke')}</button>
    </div>)}
    {failed && <p role="status" className="m-0 text-muted">{t('helper-grants-failed')}</p>}
  </div>;
}
