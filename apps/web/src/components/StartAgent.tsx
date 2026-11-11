import { useEffect, useState } from 'react';
import { t } from '../i18n';
import type { MachineModel, ProjectChoice } from '../net/view';
import type { DirectoryInfo } from '../net/directory';
export interface KindChoice { kind: string; name: string; installed: boolean }

export function StartAgent({ projects, machines, loadKinds, inspect, onStart, onClose }: {
  projects: readonly ProjectChoice[];
  machines: readonly MachineModel[];
  loadKinds: (node: string) => Promise<KindChoice[]>;
  inspect: (node: string, path: string, createIdentity: boolean) => Promise<DirectoryInfo>;
  onStart: (node: string, info: DirectoryInfo, kind: string, separate: boolean, acceptIdentityChange: boolean) => Promise<string | undefined>;
  onClose: () => void;
}) {
  const online = machines.filter(m => m.online);
  const initialProject = projects.find(p => online.some(m => m.id === p.node));
  const [node, setNode] = useState(initialProject?.node ?? online[0]?.id ?? '');
  const [path, setPath] = useState(initialProject?.path ?? '');
  const [kinds, setKinds] = useState<KindChoice[] | 'loading' | 'failed'>('loading');
  const [kind, setKind] = useState('');
  const [info, setInfo] = useState<DirectoryInfo>();
  const [createIdentity, setCreateIdentity] = useState(false);
  const [separate, setSeparate] = useState(false);
  const [acceptIdentityChange, setAcceptIdentityChange] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => {
    if (!node) return;
    let current = true;
    setKinds('loading'); setKind(''); setInfo(undefined); setError(undefined);
    loadKinds(node).then(k => { if (current) { setKinds(k); setKind(k.find(x => x.installed)?.kind ?? ''); } }, () => { if (current) setKinds('failed'); });
    return () => { current = false; };
  }, [node, loadKinds]);
  const changePath = (value: string) => { setPath(value); setInfo(undefined); setError(undefined); setSeparate(false); setAcceptIdentityChange(false); };
  const choose = async () => {
    setBusy(true); setError(undefined);
    try { setInfo(await inspect(node, path, createIdentity)); }
    catch (e) { setError(t((e as { code?: string }).code === 'approval.signature_invalid' ? 'approval-not-trusted' : (e as { code?: string }).code === 'protocol.capability_missing' ? 'start-directory-unsupported' : 'start-directory-failed')); }
    finally { setBusy(false); }
  };
  const go = async () => {
    if (!info) return;
    setBusy(true); setError(await onStart(node, info, kind, separate, acceptIdentityChange)); setBusy(false);
  };
  if (!online.length) return <section className="start"><p>{t('start-no-machines')}</p><button className="button" onClick={onClose}>{t('start-cancel')}</button></section>;
  return <section className="start" aria-label={t('start-title')}>
    <h2>{t('start-title')}</h2>
    <label><span>{t('start-machine')}</span><select value={node} disabled={busy} onChange={e => { setNode(e.target.value); changePath(projects.find(p => p.node === e.target.value)?.path ?? ''); }}>{online.map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select></label>
    {projects.some(p => p.node === node) && <label><span>{t('start-project')}</span><select value={projects.some(p => p.node === node && p.path === path) ? path : ''} disabled={busy} onChange={e => changePath(e.target.value)}><option value="">{t('start-other-folder')}</option>{projects.filter(p => p.node === node).map(p => <option key={p.path} value={p.path}>{p.label}</option>)}</select></label>}
    <label><span>{t('start-directory')}</span><input value={path} disabled={busy} onChange={e => changePath(e.target.value)} placeholder={t('start-directory-placeholder')} /></label>
    {!info && <>
      <label className="text-sm"><span><input type="checkbox" checked={createIdentity} disabled={busy} onChange={e => setCreateIdentity(e.target.checked)} /> {t('start-portable')}</span></label>
      {createIdentity && <p className="muted text-sm">{t('start-portable-explain')}</p>}
      <button className="button button--primary" disabled={!path.trim() || busy} onClick={() => void choose()}>{t('start-select-directory')}</button>
    </>}
    {info && <>
      <p className="break-all text-sm">{info.path}</p>
      <p className="muted text-sm">{t('start-in-directory')}</p>
      {info.portable && <p className="break-all text-xs text-muted">{t('start-project-id', { id: info.projectId })}</p>}
      <label><span>{t('start-which')}</span>{kinds === 'loading' ? <span>{t('start-loading')}</span> : kinds === 'failed' ? <span role="alert">{t('start-kinds-failed')}</span> : <select value={kind} disabled={busy} onChange={e => setKind(e.target.value)}>{kinds.map(k => <option key={k.kind} value={k.kind} disabled={!k.installed}>{k.installed ? k.name : `${k.name} — ${t('start-not-installed')}`}</option>)}</select>}</label>
      {info.gitRoot && <label><span><input type="checkbox" checked={separate} disabled={busy} onChange={e => setSeparate(e.target.checked)} /> {t('start-separate-worktree')}</span></label>}
      {info.previousProjectId && <label><span><input type="checkbox" checked={acceptIdentityChange} disabled={busy} onChange={e => setAcceptIdentityChange(e.target.checked)} /> {t('start-identity-change')}</span></label>}
      <button className="button button--primary" disabled={busy || !kind || (!!info.previousProjectId && !acceptIdentityChange)} onClick={() => void go()}>{t('start-go')}</button>
    </>}
    {error && <p role="alert">{error}</p>}
    <button className="button" disabled={busy} onClick={onClose}>{t('start-cancel')}</button>
  </section>;
}
