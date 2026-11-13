import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useRef, useState } from 'react';
import { t } from '../i18n';
import type { MachineModel, ProjectChoice } from '../net/view';
import { childPath, type DirectoryInfo, type DirectoryListing } from '../net/directory';
import { Icon } from './Icon';
import { machineFor } from './startChoice';
export interface KindChoice { kind: string; name: string; installed: boolean }

const chip = 'inline-flex items-center gap-2 rounded-xs border px-2.5 py-1.5 text-[13px] transition-colors duration-150 disabled:opacity-50';
const chipOn = 'border-accent bg-surface-2 text-ink ring-1 ring-accent';
const chipOff = 'border-line bg-surface-0 text-muted hover:bg-surface-2 hover:text-ink';

// "Start an agent" in one window: the machine, the folder (a known one in a click, or any path), then the agent. Choosing the
// folder shows the folder the node resolved before anything starts (ADR-0052); the rest is a click.
export function StartAgent({ projects, machines, initialNode, loadKinds, inspect, list, onStart, onClose }: {
  projects: readonly ProjectChoice[];
  machines: readonly MachineModel[];
  initialNode?: string;
  loadKinds: (node: string) => Promise<KindChoice[]>;
  inspect: (node: string, path: string, createIdentity: boolean) => Promise<DirectoryInfo>;
  // ADR-0053: the sub-folders of a folder on the machine (empty path: its home folder). Missing, or refused by an older machine: the path is typed.
  list?: (node: string, path: string) => Promise<DirectoryListing>;
  // A failure is a Fluent message id, translated when rendered so an open form follows a language change.
  onStart: (node: string, info: DirectoryInfo, kind: string, separate: boolean, acceptIdentityChange: boolean) => Promise<string | undefined>;
  onClose: () => void;
}) {
  const online = machines.filter(m => m.online);
  const preset = initialNode && online.some(m => m.id === initialNode) ? initialNode : undefined;
  const initialProject = projects.find(p => p.node === (preset ?? online[0]?.id)) ?? projects.find(p => online.some(m => m.id === p.node));
  const [node, setNode] = useState(preset ?? initialProject?.node ?? online[0]?.id ?? '');
  const [path, setPath] = useState(initialProject && initialProject.node === (preset ?? initialProject.node) ? initialProject.path ?? '' : '');
  const [kinds, setKinds] = useState<KindChoice[] | 'loading' | 'failed'>('loading');
  const [kind, setKind] = useState('');
  const [info, setInfo] = useState<DirectoryInfo>();
  const [createIdentity, setCreateIdentity] = useState(false);
  const [separate, setSeparate] = useState(false);
  const [acceptIdentityChange, setAcceptIdentityChange] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // The folder picker: what the machine answered for the folder being looked at.
  const [browse, setBrowse] = useState<{ listing?: DirectoryListing; busy: boolean; failed?: boolean } | undefined>();
  const [canBrowse, setCanBrowse] = useState(!!list);
  const look = async (at: string) => {
    if (!list) return;
    setBrowse((b) => ({ listing: b?.listing, busy: true }));
    try { setBrowse({ listing: await list(node, at), busy: false }); }
    catch (e) {
      // A machine without the command: keep the typed path, and do not offer the picker again.
      if ((e as { code?: string }).code === 'protocol.capability_missing') { setCanBrowse(false); setBrowse(undefined); }
      else setBrowse((b) => ({ listing: b?.listing, busy: false, failed: true }));
    }
  };
  useEffect(() => { setBrowse(undefined); setCanBrowse(!!list); }, [node, list]);
  useEffect(() => {
    if (!node) return;
    let current = true;
    setKinds('loading'); setKind(''); setInfo(undefined); setError(undefined);
    loadKinds(node).then(k => { if (current) { setKinds(k); setKind(k.find(x => x.installed)?.kind ?? ''); } }, () => { if (current) setKinds('failed'); });
    return () => { current = false; };
  }, [node, loadKinds]);
  const changePath = (value: string) => { setPath(value); setInfo(undefined); setError(undefined); setSeparate(false); setAcceptIdentityChange(false); };
  const changeNode = (id: string) => { setNode(id); changePath(projects.find(p => p.node === id)?.path ?? ''); };
  // The machine and its project can reach the view after the window opened (a node that has just dialed in). Then the window
  // selects that machine, and fills in its project folder unless the user has typed a path of their own.
  const typed = useRef(false);
  const onlineKey = online.map(m => m.id).join('\n');
  useEffect(() => {
    const next = machineFor(node, online.map(m => m.id), initialNode, projects);
    if (next === node) return;
    setNode(next);
    if (!typed.current) changePath(projects.find(p => p.node === next)?.path ?? '');
    // Keyed on which machines are online, not on every new array.
  }, [onlineKey]);
  const projectPath = projects.find(p => p.node === node)?.path ?? '';
  useEffect(() => {
    if (!typed.current && !path && !info && projectPath) setPath(projectPath);
    // Only when the machine's project path appears or changes.
  }, [projectPath]);
  const choose = async (at = path) => {
    setBusy(true); setError(undefined);
    try { setInfo(await inspect(node, at, createIdentity)); }
    catch (e) { setError((e as { code?: string }).code === 'approval.signature_invalid' ? 'approval-not-trusted' : (e as { code?: string }).code === 'protocol.capability_missing' ? 'start-directory-unsupported' : 'start-directory-failed'); }
    finally { setBusy(false); }
  };
  const go = async () => {
    if (!info) return;
    setBusy(true); setError(await onStart(node, info, kind, separate, acceptIdentityChange)); setBusy(false);
  };
  const known = projects.flatMap(p => (p.node === node && p.path ? [{ ...p, path: p.path }] : []));
  const machineName = (m: MachineModel) => (m.name === m.id ? m.name : t('machine-name-id', { name: m.name, id: m.id }));

  return <Dialog.Root open onOpenChange={open => { if (!open && !busy) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
      <Dialog.Content aria-describedby={undefined} aria-label={t('start-title')}
        className="fixed left-1/2 top-[8vh] z-50 flex max-h-[84vh] w-[min(36rem,calc(100vw-1.5rem))] -translate-x-1/2 flex-col overflow-hidden rounded-m border border-line bg-surface-1 shadow-card">
        <div className="flex items-center gap-2 border-b border-line px-4 py-3">
          <Icon name="terminal" className="size-4 text-accent" />
          <Dialog.Title className="m-0 flex-1 text-sm font-semibold tracking-normal text-ink normal-case">{t('start-title')}</Dialog.Title>
          <Dialog.Close className="ide-iconbtn" aria-label={t('start-cancel')} disabled={busy}><Icon name="close" /></Dialog.Close>
        </div>
        {!online.length ? <div className="p-4"><p className="m-0">{t('start-no-machines')}</p></div> : <div className="grid min-h-0 gap-4 overflow-y-auto p-4">
          <div className="grid gap-1.5">
            <h2 className="m-0">{t('start-machine')}</h2>
            <div className="flex flex-wrap gap-1.5" role="radiogroup" aria-label={t('start-machine')}>
              {online.map(m => <button key={m.id} type="button" role="radio" aria-checked={node === m.id} disabled={busy} className={`${chip} ${node === m.id ? chipOn : chipOff}`} onClick={() => node !== m.id && changeNode(m.id)}>
                <Icon name="machine" className="size-3.5" />{machineName(m)}
              </button>)}
            </div>
          </div>

          <div className="grid gap-1.5">
            <h2 className="m-0">{t('ide-folder-chosen')}</h2>
            {info ? <div className="flex items-start gap-2 rounded-xs border border-line bg-surface-0 px-3 py-2">
              <Icon name="folder" className="mt-0.5 size-4 text-accent" />
              <div className="min-w-0 flex-1">
                <p className="m-0 break-all font-mono text-[13px]">{info.path}</p>
                <p className="m-0 mt-0.5 text-xs text-muted">{info.branch ? <><Icon name="branch" className="me-1 inline size-3" />{info.branch} · </> : null}{t('start-in-directory')}</p>
                {info.portable && <p className="m-0 mt-0.5 break-all text-xs text-muted">{t('start-project-id', { id: info.projectId })}</p>}
              </div>
              <button type="button" className="button px-2 py-1 text-xs" disabled={busy} onClick={() => setInfo(undefined)}>{t('ide-folder-change')}</button>
            </div> : <>
              <div className="flex gap-2">
                <input value={path} disabled={busy} autoFocus className="min-w-0 flex-1 font-mono text-[13px]" aria-label={t('start-directory')} onChange={e => { typed.current = true; changePath(e.target.value); }} onKeyDown={e => { if (e.key === 'Enter' && path.trim() && !busy) void choose(); }} placeholder={t('start-directory-placeholder')} />
                <button type="button" className="button button--primary" disabled={!path.trim() || busy} onClick={() => void choose()}>{t('start-select-directory')}</button>
              </div>
              {canBrowse && !browse && <button type="button" className="button justify-self-start px-3 py-1 text-xs" disabled={busy} onClick={() => void look(path.trim().startsWith('/') || /^[A-Za-z]:[\\/]/.test(path.trim()) ? path.trim() : '')}><Icon name="folder" className="size-4" />{t('start-browse')}</button>}
              {browse && <div className="grid gap-1 rounded-xs border border-line bg-surface-0 p-2" aria-busy={browse.busy}>
                <div className="flex items-center gap-1.5">
                  <button type="button" className="ide-iconbtn" disabled={!browse.listing?.parent || browse.busy} onClick={() => void look(browse.listing?.parent ?? '')} aria-label={t('start-browse-up')} title={t('start-browse-up')}><Icon name="chevron" className="size-4 rotate-180 rtl:rotate-0" /></button>
                  <code dir="ltr" className="min-w-0 flex-1 truncate text-xs" title={browse.listing?.path}>{browse.listing?.path ?? '…'}</code>
                  <button type="button" className="button button--primary px-2 py-1 text-xs" disabled={!browse.listing || busy} onClick={() => { const p = browse.listing?.path ?? ''; setBrowse(undefined); changePath(p); void choose(p); }}>{t('start-select-directory')}</button>
                  <button type="button" className="ide-iconbtn" onClick={() => setBrowse(undefined)} aria-label={t('start-browse-close')} title={t('start-browse-close')}><Icon name="close" className="size-4" /></button>
                </div>
                {browse.failed && <p role="alert" className="m-0 text-xs text-needs-you">{t('start-browse-failed')}</p>}
                <ul className="m-0 grid max-h-52 list-none gap-0.5 overflow-y-auto p-0">
                  {browse.listing?.entries.length === 0 && <li className="muted px-2 py-1 text-xs">{t('start-browse-empty')}</li>}
                  {browse.listing?.entries.map((e) => <li key={e.name}><button type="button" className="ide-row py-1" disabled={browse.busy} onClick={() => void look(childPath(browse.listing?.path ?? '', e.name))}>
                    <Icon name="folder" className="size-4 text-muted" /><span className="min-w-0 flex-1 truncate">{e.name}</span>{e.isGit && <span className="ide-chip" title={t('start-browse-git')}><Icon name="branch" className="size-3" />git</span>}
                  </button></li>)}
                </ul>
                {browse.listing?.truncated && <p className="muted m-0 text-xs">{t('start-browse-more')}</p>}
              </div>}
              {known.length > 0 && !browse && <div className="grid gap-0.5">
                <span className="text-[11px] text-muted">{t('ide-folder-known')}</span>
                <ul className="m-0 grid max-h-44 list-none gap-0.5 overflow-y-auto p-0">
                  {known.map(p => <li key={p.path}><button type="button" className="ide-row py-1.5" disabled={busy} aria-current={p.path === path ? 'true' : undefined} onClick={() => { changePath(p.path); void choose(p.path); }}>
                    <Icon name="folder" className="size-4 text-muted" /><span className="min-w-0 flex-1"><span className="block truncate">{p.label}</span><span className="block truncate font-mono text-[11px] text-muted">{p.path}</span></span>
                  </button></li>)}
                </ul>
              </div>}
              <label className="flex items-start gap-2 text-[13px]"><input type="checkbox" className="mt-0.5" checked={createIdentity} disabled={busy} onChange={e => setCreateIdentity(e.target.checked)} /><span>{t('start-portable')}{createIdentity && <span className="mt-0.5 block text-xs text-muted">{t('start-portable-explain')}</span>}</span></label>
            </>}
          </div>

          {info && <>
            <div className="grid gap-1.5">
              <h2 className="m-0">{t('ide-agent-pick')}</h2>
              {kinds === 'loading' ? <p className="muted m-0">{t('start-loading')}</p> : kinds === 'failed' ? <p role="alert" className="m-0">{t('start-kinds-failed')}</p> : kinds.length === 0 ? <p className="muted m-0">{t('start-no-kinds')}</p> :
                <div className="flex flex-wrap gap-1.5" role="radiogroup" aria-label={t('start-which')}>
                  {kinds.map(k => <button key={k.kind} type="button" role="radio" aria-checked={kind === k.kind} disabled={busy || !k.installed} className={`${chip} ${kind === k.kind ? chipOn : chipOff}`} onClick={() => setKind(k.kind)}>
                    {k.name}{!k.installed && <span className="text-[11px]">— {t('start-not-installed')}</span>}
                  </button>)}
                </div>}
              {Array.isArray(kinds) && kinds.some(k => !k.installed) && <p className="m-0 text-xs text-muted">{t('start-not-installed-why')}</p>}
            </div>
            {(info.gitRoot || info.previousProjectId) && <div className="grid gap-1.5">
              <h2 className="m-0">{t('ide-options')}</h2>
              {info.gitRoot && <label className="flex items-center gap-2 text-[13px]"><input type="checkbox" checked={separate} disabled={busy} onChange={e => setSeparate(e.target.checked)} />{t('start-separate-worktree')}</label>}
              {info.previousProjectId && <label className="flex items-center gap-2 text-[13px]"><input type="checkbox" checked={acceptIdentityChange} disabled={busy} onChange={e => setAcceptIdentityChange(e.target.checked)} />{t('start-identity-change')}</label>}
            </div>}
          </>}
          {error && <p role="alert" className="m-0 text-needs-you">{t(error)}</p>}
        </div>}
        <div className="flex justify-end gap-2 border-t border-line px-4 py-3">
          <button type="button" className="button" disabled={busy} onClick={onClose}>{t('start-cancel')}</button>
          {info && <button type="button" className="button button--primary" disabled={busy || !kind || (!!info.previousProjectId && !acceptIdentityChange)} onClick={() => void go()}>{t('start-go')}</button>}
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
