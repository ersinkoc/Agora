import { useEffect, useState } from 'react';
import { t } from '../i18n';
import type { ProjectChoice } from '../net/view';

export interface KindChoice { kind: string; name: string; installed: boolean }

// Start an agent in three plain choices: where (machine and project), which agent, go. Kinds come from the chosen
// machine itself (ADR-0030); one that is not installed there is shown and cannot be picked.
export function StartAgent({ projects, loadKinds, onStart, onClose }: {
  projects: readonly ProjectChoice[];
  loadKinds: (node: string) => Promise<KindChoice[]>;
  onStart: (node: string, projectId: string, kind: string) => Promise<string | undefined>; // an error message, or undefined on success
  onClose: () => void;
}) {
  const [where, setWhere] = useState(0);
  const [kinds, setKinds] = useState<KindChoice[] | 'loading' | 'failed'>('loading');
  const [kind, setKind] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const project = projects[where];
  const node = project?.node;

  useEffect(() => {
    if (!node) return;
    let live = true;
    setKinds('loading');
    setKind('');
    loadKinds(node).then(
      (k) => { if (live) { setKinds(k); setKind(k.find((x) => x.installed)?.kind ?? ''); } },
      () => { if (live) setKinds('failed'); },
    );
    return () => { live = false; };
  }, [node, loadKinds]);

  if (!project) {
    return (
      <section className="start" aria-label={t('start-title')}>
        <h2>{t('start-title')}</h2>
        <p className="muted">{t('start-no-projects')}</p>
        <button type="button" className="button" onClick={onClose}>{t('start-cancel')}</button>
      </section>
    );
  }
  const go = async () => {
    setBusy(true);
    setError(await onStart(project.node, project.projectId, kind));
    setBusy(false);
  };
  return (
    <section className="start" aria-label={t('start-title')}>
      <h2>{t('start-title')}</h2>
      <label>
        <span className="muted">{t('start-where')}</span>{' '}
        <select value={where} onChange={(e) => setWhere(Number(e.target.value))}>
          {projects.map((p, i) => <option key={`${p.node}/${p.projectId}`} value={i}>{p.label}</option>)}
        </select>
      </label>
      <label>
        <span className="muted">{t('start-which')}</span>{' '}
        {kinds === 'loading' ? <span className="muted">{t('start-loading')}</span>
          : kinds === 'failed' ? <span role="alert">{t('start-kinds-failed')}</span>
          : kinds.length === 0 ? <span className="muted">{t('start-no-kinds')}</span>
          : (
            <select value={kind} onChange={(e) => setKind(e.target.value)}>
              {kinds.map((k) => (
                <option key={k.kind} value={k.kind} disabled={!k.installed}>
                  {k.installed ? k.name : `${k.name} — ${t('start-not-installed')}`}
                </option>
              ))}
            </select>
          )}
      </label>
      {error && <p role="alert">{error}</p>}
      <div className="start__actions">
        <button type="button" className="button button--primary" disabled={busy || !kind} onClick={go}>{t('start-go')}</button>
        <button type="button" className="button" onClick={onClose}>{t('start-cancel')}</button>
      </div>
    </section>
  );
}
