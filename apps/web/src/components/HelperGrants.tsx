import { useCallback, useEffect, useRef, useState } from 'react';
import { t } from '../i18n';

// A live "more helpers like this" permission of one agent (ADR-0057), as its machine lists it.
export interface HelperGrant { id: string; kind: string; folder: string; remaining: number }

// ADR-0057: what an agent may still start without asking, with a way to take it back. Shown for an agent that has helpers.
export function HelperGrants({ list, revoke, refresh }: {
  list: () => Promise<HelperGrant[]>;
  revoke: (id: string) => Promise<void>;
  refresh: number; // changes when the agent's helpers change, so the remaining count is read again
}) {
  const [grants, setGrants] = useState<HelperGrant[]>([]);
  const [failed, setFailed] = useState(false);
  const gen = useRef(0);
  const load = useCallback(() => {
    const current = ++gen.current;
    list()
      .then((g) => { if (gen.current === current) { setGrants(g); setFailed(false); } })
      .catch(() => { if (gen.current === current) setFailed(true); });
  }, [list]);
  useEffect(() => {
    load();
    return () => { gen.current += 1; };
  }, [load, refresh]);
  // A revoke belongs to the view it was made in (this agent's list), not to the list read that was current when it was clicked: every
  // revoke that succeeds while the view lasts reads the list again, even when another revoke's read came in between.
  const scope = useRef(0);
  useEffect(() => () => { scope.current += 1; }, [list]);
  if (grants.length === 0 && !failed) return null;
  return <div className="flex flex-col gap-1 border-b border-line bg-surface-1 px-3 py-2 text-xs" aria-label={t('helper-grants')}>
    {grants.map((g) => <div key={g.id} className="flex flex-wrap items-center gap-2">
      <span className="min-w-0 flex-1">{t('helper-grant-line', { count: g.remaining, kind: g.kind })} <code dir="ltr" className="break-all text-muted">{g.folder}</code></span>
      <button type="button" className="button px-2 py-1 text-xs" onClick={() => {
        const view = scope.current;
        void revoke(g.id).then(
          () => { if (scope.current === view) load(); },
          () => { if (scope.current === view) setFailed(true); },
        );
      }}>{t('helper-grant-revoke')}</button>
    </div>)}
    {failed && <p role="status" className="m-0 text-muted">{t('helper-grants-failed')}</p>}
  </div>;
}
