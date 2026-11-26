import { StartLimitHint } from './Quota';
import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useRef, useState } from 'react';
import { t } from '../i18n';
import { sharesName } from '../model';
import type { MachineModel, ProjectChoice } from '../net/view';
import { childPath, directoryReasonKey, type DirectoryInfo, type DirectoryListing } from '../net/directory';
import { ROLE_REFUSAL, refusalCode } from '../net/startRefusal';
import { isRegistryKind } from '../net/agentKinds';
import { Icon, type IconName } from './Icon';
import { AgentAvatar, Button, Callout, SegmentedControl } from './kit';
import { KindCards, NO_COLOUR } from './StartKinds';
import { isBareName, machineFor, missingNote, shortBinary, trustCommand } from './startChoice';
import { LaunchSettings } from './LaunchSettings';
import { recall, remember } from './launchMemory';
import { chosenSettings, noSettings, settingName, usableSettings, type LaunchSettingChoice, type SettingsState } from './launchSettingsModel';
import { ASK, chosenIds, confirmedIds, confirmSkip, harmless, initialLaunch, keepAsking, labelOf, pickPermission, skipping, toggleHarmless, usableOptions, type LaunchOptionChoice } from './launchChoice';
// ADR-0059: `options` are the kind's launch options as the machine lists them (none from an older machine).
// ADR-0066: `settings` are the kind's launch settings as the machine lists them (none from an older machine).
// #903: a refused start in words; a missing program is named, in the words the "not installed" note uses, when the machine said which.
export function refusalWords(error: string, program: string | undefined): [string, Record<string, string>?] {
  return error === 'start-program-missing' && program ? ['start-not-installed-program', { program }] : [error];
}

export interface KindChoice { kind: string; name: string; installed: boolean; level?: number; options?: LaunchOptionChoice[]; program?: string; settings?: LaunchSettingChoice[] }

// The command a machine's owner runs once so the machine trusts this device (ADR-0020), with a Copy button: no hunting for it on another page.
function TrustCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return <div className="flex items-center gap-2 rounded-s border border-line bg-surface-0 px-3 py-2">
    <code className="min-w-0 flex-1 break-all font-mono text-caption">{command}</code>
    <Button size="sm" onClick={() => { void navigator.clipboard?.writeText(command).then(() => setCopied(true), () => undefined); }}><Icon name="copy" className="size-3.5" />{copied ? t('trust-copied') : t('trust-copy')}</Button>
  </div>;
}

// ADR-0061: another device of this person that the machine trusts can approve this one; asking puts a card on that device.
function AskApproval({ ask }: { ask: () => Promise<boolean> }) {
  const [state, setState] = useState<'idle' | 'busy' | 'asked' | 'failed'>('idle');
  return <div className="flex flex-wrap items-center gap-2">
    <Button variant="primary" disabled={state === 'busy' || state === 'asked'}
      onClick={() => { setState('busy'); void ask().then((ok) => setState(ok ? 'asked' : 'failed')); }}>{t('trust-ask')}</Button>
    {state === 'asked' && <span role="status" className="text-body text-muted">{t('trust-asked')}</span>}
    {state === 'failed' && <span role="alert" className="text-body text-danger">{t('device-ask-failed')}</span>}
  </div>;
}

// A section title: sentence case at the strong size (visual-language.md §2), not the uppercase micro-heading of a bare h2.
const H = 'm-0 text-strong font-semibold normal-case tracking-normal text-ink';
// The machine's system beside its name (the same outlines as My machines); a system this build has no outline for gets the plain machine.
const OS_ICON: Record<string, IconName> = { linux: 'osLinux', macos: 'osMac', windows: 'osWindows' };

// "Will appear as codex · …": the kind's id, as the agent's name begins on Home and in the tabs (`kind · colour`), with the name in bold.
// The colour comes from the id the machine gives the agent when it starts, so it is not shown before.
export function AppearsAs({ kind }: { kind: string }) {
  const MARK = '\u0001';
  const [before = '', after = ''] = t('start-preview', { name: MARK }).split(MARK);
  return <p data-start-preview="" className="m-0 flex min-w-0 items-center gap-2 text-caption text-muted">
    <AgentAvatar accent={NO_COLOUR} kind={kind} size="s" />
    <span className="min-w-0">{before}<strong className="font-semibold text-ink"><bdi>{kind} · …</bdi></strong>{after}</span>
  </p>;
}

// "Start an agent" in one window: the machine, the folder (a known one in a click, or any path), then the agent. Choosing the
// folder shows the folder the node resolved before anything starts (ADR-0052); the rest is a click.
export function StartAgent({ projects, machines, initialNode, initialKind, forTask, trust, askApproval, loadKinds, inspect, list, onStart, onClose }: {
  projects: readonly ProjectChoice[];
  machines: readonly MachineModel[];
  initialNode?: string;
  // A kind to start on (a plain terminal asks for `shell`); when the machine offers no such kind nothing is chosen for the person.
  initialKind?: string;
  // ADR-0095 step 3: the start is for a board task. The window names the task, offers agents only (a plain terminal cannot take a task),
  // and starts only in a folder of the task's project, since the task can only be given to an agent of its own project. The caller passes
  // the task's project folders as `projects`, so the machine and folder are prefilled exactly as an ordinary start prefills them.
  forTask?: { projectId: string; project: string; title: string };
  // This device's id and public key: when a machine does not trust the device yet, the command that fixes it is shown right here.
  trust?: { id: string; key: string };
  // ADR-0061: whether another device of this person that `node` trusts could approve this one, and asking it to.
  askApproval?: { can: (node: string) => boolean; ask: () => Promise<boolean> };
  loadKinds: (node: string) => Promise<KindChoice[]>;
  inspect: (node: string, path: string, createIdentity: boolean) => Promise<DirectoryInfo>;
  // ADR-0053: the sub-folders of a folder on the machine (empty path: its home folder). Missing, or refused by an older machine: the path is typed.
  list?: (node: string, path: string) => Promise<DirectoryListing>;
  // A failure is a Fluent message id, translated when rendered so an open form follows a language change.
  onStart: (node: string, info: DirectoryInfo, kind: string, separate: boolean, acceptIdentityChange: boolean, launch: { ids: string[]; confirmed: string[]; settings: [string, string][] }) => Promise<string | undefined>;
  onClose: () => void;
}) {
  const online = machines.filter(m => m.online);
  const preset = initialNode && online.some(m => m.id === initialNode) ? initialNode : undefined;
  const initialProject = projects.find(p => p.node === (preset ?? online[0]?.id)) ?? projects.find(p => online.some(m => m.id === p.node));
  const [node, setNode] = useState(preset ?? initialProject?.node ?? online[0]?.id ?? '');
  // ADR-0069: where the chosen machine's program runs from, when that folder is not on its PATH.
  const binary = online.find(m => m.id === node)?.binary || undefined;
  const machineOs = online.find(m => m.id === node)?.os;
  // The whole path only when the person asks for it (it can hold the machine's user name); a member never gets more than two parts.
  const [wholePath, setWholePath] = useState(false);
  const [path, setPath] = useState(initialProject && initialProject.node === (preset ?? initialProject.node) ? initialProject.path ?? '' : '');
  const [kinds, setKinds] = useState<KindChoice[] | 'loading' | 'failed'>('loading');
  const [kind, setKind] = useState('');
  const [info, setInfo] = useState<DirectoryInfo>();
  const [createIdentity, setCreateIdentity] = useState(false);
  const [separate, setSeparate] = useState(false);
  const [acceptIdentityChange, setAcceptIdentityChange] = useState(false);
  // ADR-0059: "Ask before acting" until the user picks an option that skips permissions and confirms it; a new agent kind starts over.
  const [launch, setLaunch] = useState(initialLaunch);
  useEffect(() => {
    setLaunch(initialLaunch());
    // ADR-0066 decision 3: what was last chosen for this kind on this machine, cleaned against what the machine lists now.
    const listed = kinds !== 'loading' && kinds !== 'failed' ? kinds.find((k) => k.kind === kind)?.settings : undefined;
    setChosen(kind && listed ? recall(node, kind, listed) : noSettings);
  }, [kind, node, kinds]); // eslint-disable-line react-hooks/exhaustive-deps
  // ADR-0066: a value per setting, or nothing (the agent's own default); a new kind or machine starts over.
  const [chosen, setChosen] = useState<SettingsState>(noSettings);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // The folder picker: what the machine answered for the folder being looked at.
  const [browse, setBrowse] = useState<{ listing?: DirectoryListing; busy: boolean; failed?: boolean; reason?: string } | undefined>();
  const [canBrowse, setCanBrowse] = useState(!!list);
  const lookGen = useRef(0);
  const look = async (at: string) => {
    if (!list) return;
    const gen = ++lookGen.current;
    setBrowse((b) => ({ listing: b?.listing, busy: true }));
    try {
      const listing = await list(node, at);
      if (lookGen.current === gen) setBrowse({ listing, busy: false });
    }
    catch (e) {
      if (lookGen.current !== gen) return;
      // A machine without the command: keep the typed path, and do not offer the picker again.
      if ((e as { code?: string }).code === 'protocol.capability_missing') { setCanBrowse(false); setBrowse(undefined); }
      else {
        // A lasting refusal (this device limited to other projects, or this person's role, #1044) says so; anything else says it failed.
        const why = refusalCode(e, 'DirectoryList');
        setBrowse((b) => ({ listing: b?.listing, busy: false, failed: true, reason: why === 'auth.forbidden' || why === ROLE_REFUSAL ? directoryReasonKey(why) : 'start-browse-failed' }));
      }
    }
  };
  useEffect(() => { lookGen.current++; setBrowse(undefined); setCanBrowse(!!list); }, [node, list]);
  useEffect(() => {
    if (!node) return;
    let current = true;
    setKinds('loading'); setKind(''); setInfo(undefined); setError(undefined);
    loadKinds(node).then(all => { if (current) { const k = forTask ? all.filter(x => x.kind !== 'shell') : all; setKinds(k); setKind(k.find(x => x.installed && x.kind === initialKind)?.kind ?? (initialKind ? '' : k.find(x => x.installed && !isRegistryKind(x.kind))?.kind ?? '')); } }, () => { if (current) setKinds('failed'); });
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
    // For a task the project is the task's: no new project identity is made for the folder (it would make it another project).
    try { setInfo(await inspect(node, at, forTask ? false : createIdentity)); }
    catch (e) { setError(directoryReasonKey(refusalCode(e, 'DirectoryInspect'))); }
    finally { setBusy(false); }
  };
  const go = async () => {
    if (!info || !chosenKind?.installed) return;
    setBusy(true);
    const failure = await onStart(node, info, kind, separate, acceptIdentityChange, { ids: chosenIds(launch, offered), confirmed: confirmedIds(launch, offered), settings: sending });
    if (!failure) remember(node, kind, sending, takes);
    setError(failure); setBusy(false);
  };
  // The chosen kind; one the machine cannot find can be chosen (to read why) but offers nothing and never starts.
  const chosenKind = Array.isArray(kinds) ? kinds.find((k) => k.kind === kind) : undefined;
  const startable = chosenKind?.installed ? chosenKind : undefined;
  const missing = missingNote(chosenKind);
  // Ids outside `[a-z0-9-]` are never offered (the hub relays the list; the machine decides).
  const offered: LaunchOptionChoice[] = usableOptions(startable?.options ?? []);
  // ADR-0066: only what the machine listed, only values it allows; empty when nothing is chosen (then the start is the one without settings).
  const takes = usableSettings(startable?.settings ?? []);
  const sending = chosenSettings(chosen, takes);
  const skips = skipping(offered), plain = harmless(offered);
  const pendingOption = offered.find((o) => o.id === launch.pending);
  // The same folder can be spelled several ways on Windows (`D:\x`, `D:/x`, `\\?\D:\x`): listed once.
  const sameFolder = (path: string) => path.replace(/^\\\\\?\\/, '').replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  const seen = new Set<string>();
  const known = projects.flatMap(p => (p.node === node && p.path && !seen.has(sameFolder(p.path)) && seen.add(sameFolder(p.path)) ? [{ ...p, path: p.path }] : []));
  const machineName = (m: MachineModel) => (sharesName(m, machines) ? t('machine-name-id', { name: m.name, id: m.id }) : m.name);
  // A folder of another project: the task could not be given to an agent started there.
  const otherProject = !!forTask && !!info && info.projectId !== forTask.projectId;

  const kindName = (k: string) => (Array.isArray(kinds) ? kinds.find((x) => x.kind === k)?.name ?? k : k);
  const machineLabel = online.find(m => m.id === node)?.name ?? node;
  const risky = launch.permission !== ASK;

  return <Dialog.Root open onOpenChange={open => { if (!open && !busy) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
      <Dialog.Content aria-describedby={undefined} aria-label={t('start-title')}
        className="fixed left-1/2 top-[8vh] z-50 flex max-h-[84vh] w-[min(36rem,calc(100vw-1.5rem))] -translate-x-1/2 flex-col overflow-hidden rounded-l border border-line bg-surface-raised shadow-pop">
        <div className="flex items-center gap-2.5 px-5 pb-1 pt-4">
          <span aria-hidden className="kit-chip kit-tone-accent"><Icon name="terminal" /></span>
          <Dialog.Title className="m-0 flex-1 text-title font-semibold tracking-normal text-ink normal-case">{t('start-title')}</Dialog.Title>
          <Dialog.Close className="ide-iconbtn" aria-label={t('start-cancel')} disabled={busy}><Icon name="close" /></Dialog.Close>
        </div>
        {!online.length ? <div className="px-5 py-4"><p className="m-0">{t('start-no-machines')}</p></div> : <div className="grid min-h-0 gap-5 overflow-y-auto px-5 py-4">
          {forTask && <p data-start-for-task="" dir="auto" className="m-0 break-words rounded-m border border-line-accent bg-tint-accent px-3 py-2.5 text-body text-ink">{t('start-for-task', { title: forTask.title, project: forTask.project })}</p>}
          <div className="grid gap-2">
            <h2 className={H}>{t('start-machine')}</h2>
            {/* Many machines shrink their segments (each shows its whole name as its tooltip) rather than scroll. */}
            <div>
              <SegmentedControl label={t('start-machine')} value={node} disabled={busy} onChange={(id) => { if (id !== node) changeNode(id); }}
                items={online.map((m) => ({ value: m.id, label: machineName(m), title: machineName(m), icon: <Icon name={OS_ICON[m.os] ?? 'machine'} className="size-3.5" /> }))} />
            </div>
          </div>

          <div className="grid gap-2">
            <h2 className={H}>{t('ide-folder-chosen')}</h2>
            {info ? <div className="kit-card kit-card--dense flex items-start gap-3">
              <span aria-hidden className="kit-chip kit-tone-accent"><Icon name="folder" /></span>
              <div className="min-w-0 flex-1">
                <p className="m-0 break-all font-mono text-body text-ink">{info.path}</p>
                <p className="m-0 mt-0.5 text-caption text-muted">{info.branch ? <><Icon name="branch" className="me-1 inline size-3" />{info.branch} · </> : null}{t('start-in-directory')}</p>
                {info.portable && <p className="m-0 mt-0.5 break-all text-caption text-muted">{t('start-project-id', { id: info.projectId })}</p>}
              </div>
              <Button size="sm" disabled={busy} onClick={() => setInfo(undefined)}>{t('ide-folder-change')}</Button>
            </div> : <>
              <div className="flex gap-2">
                <input value={path} disabled={busy} autoFocus className="min-w-0 flex-1 font-mono text-body" aria-label={t('start-directory')} onChange={e => { typed.current = true; changePath(e.target.value); }} onKeyDown={e => { if (e.key === 'Enter' && path.trim() && !busy) void choose(); }} placeholder={t('start-directory-placeholder')} />
                <Button variant="primary" disabled={!path.trim() || busy} onClick={() => void choose()}>{t('start-select-directory')}</Button>
              </div>
              {canBrowse && !browse && <Button variant="ghost" size="sm" className="justify-self-start" disabled={busy} onClick={() => void look(path.trim().startsWith('/') || /^[A-Za-z]:[\\/]/.test(path.trim()) ? path.trim() : '')}><Icon name="folder" className="size-4" />{t('start-browse')}</Button>}
              {browse && <div className="grid gap-1 rounded-m border border-line bg-surface-1 p-2" aria-busy={browse.busy}>
                <div className="flex items-center gap-1.5">
                  <button type="button" className="ide-iconbtn" disabled={!browse.listing?.parent || browse.busy} onClick={() => void look(browse.listing?.parent ?? '')} aria-label={t('start-browse-up')} title={t('start-browse-up')}><Icon name="chevron" className="size-4 rotate-180 rtl:rotate-0" /></button>
                  <code dir="ltr" className="min-w-0 flex-1 truncate text-caption" title={browse.listing?.path}>{browse.listing?.path ?? '…'}</code>
                  <Button variant="primary" size="sm" disabled={!browse.listing || busy} onClick={() => { const p = browse.listing?.path ?? ''; setBrowse(undefined); changePath(p); void choose(p); }}>{t('start-select-directory')}</Button>
                  <button type="button" className="ide-iconbtn" onClick={() => setBrowse(undefined)} aria-label={t('start-browse-close')} title={t('start-browse-close')}><Icon name="close" className="size-4" /></button>
                </div>
                {browse.failed && <p role="alert" className="m-0 text-caption text-danger">{t(browse.reason ?? 'start-browse-failed')}</p>}
                <ul className="m-0 grid max-h-52 list-none gap-0.5 overflow-y-auto p-0">
                  {browse.listing?.entries.length === 0 && <li className="muted px-2 py-1 text-caption">{t('start-browse-empty')}</li>}
                  {browse.listing?.entries.map((e) => <li key={e.name}><button type="button" className="kit-row" disabled={browse.busy} onClick={() => void look(childPath(browse.listing?.path ?? '', e.name))}>
                    <Icon name="folder" className="size-4 text-muted" /><span className="kit-row__label">{e.name}</span>{e.isGit && <span className="kit-tag" title={t('start-browse-git')}><Icon name="branch" className="size-3" />git</span>}
                  </button></li>)}
                </ul>
                {browse.listing?.truncated && <p className="muted m-0 text-caption">{t('start-browse-more')}</p>}
              </div>}
              {known.length > 0 && !browse && <div className="grid gap-1">
                <span className="text-caption text-muted">{t('ide-folder-known')}</span>
                <ul className="m-0 grid max-h-44 list-none gap-0.5 overflow-y-auto p-0">
                  {known.map(p => <li key={p.path}><button type="button" className="kit-row py-1.5" disabled={busy} aria-current={p.path === path ? 'true' : undefined} onClick={() => { changePath(p.path); void choose(p.path); }}>
                    <Icon name="folder" className="size-4 text-muted" /><span className="min-w-0 flex-1"><span className="block truncate">{p.label}</span><span className="block truncate font-mono text-caption text-muted">{p.path}</span></span>
                  </button></li>)}
                </ul>
              </div>}
              {!forTask && <label className="touch-row flex items-start gap-2 text-body"><input type="checkbox" className="mt-0.5" checked={createIdentity} disabled={busy} onChange={e => setCreateIdentity(e.target.checked)} /><span>{t('start-portable')}{createIdentity && <span className="mt-0.5 block text-caption text-muted">{t('start-portable-explain')}</span>}</span></label>}
            </>}
          </div>

          {otherProject && forTask && <p role="alert" className="m-0 text-body text-danger">{t('start-for-task-other-project', { project: forTask.project })}</p>}
          {info && <>
            <div className="grid gap-2">
              <h2 className={H}>{t('ide-agent-pick')}</h2>
              {kinds === 'loading' ? <p className="muted m-0">{t('start-loading')}</p> : kinds === 'failed' ? <p role="alert" className="m-0 text-danger">{t('start-kinds-failed')}</p> : kinds.length === 0 ? <p className="muted m-0">{t('start-no-kinds')}</p> :
                <KindCards kinds={kinds} value={kind} onChange={setKind} disabled={busy} label={t('start-which')} />}
              {startable && <StartLimitHint tool={kind} machineId={node} />}
              {Array.isArray(kinds) && initialKind && !kinds.some(k => k.installed && k.kind === initialKind) && <p role="note" className="m-0 text-caption text-muted">{t('start-terminal-missing', { kind: initialKind })}</p>}
              {/* L9: why a kind cannot start, only for the one chosen. ADR-0069: its program by name; the general sentence for a kind from an
                  older machine that does not name it. */}
              {missing && <p role="note" data-start-missing="" className="m-0 rounded-m bg-surface-2 px-3 py-2 text-caption text-muted">{t(...missing)}</p>}
            </div>
            {!missing && (info.gitRoot || info.previousProjectId) && <div className="grid gap-2">
              <h2 className={H}>{t('ide-options')}</h2>
              {info.gitRoot && <label className="flex items-center gap-2 text-body"><input type="checkbox" checked={separate} disabled={busy} onChange={e => setSeparate(e.target.checked)} />{t('start-separate-worktree')}</label>}
              {info.previousProjectId && <label className="flex items-center gap-2 text-body"><input type="checkbox" checked={acceptIdentityChange} disabled={busy} onChange={e => setAcceptIdentityChange(e.target.checked)} />{t('start-identity-change')}</label>}
            </div>}
            {takes.length > 0 && <LaunchSettings settings={takes} chosen={chosen} onChange={setChosen} disabled={busy} />}
            {offered.length > 0 && <div className="grid gap-2">
              <h2 className={H}>{t('start-how-it-acts')}</h2>
              {/* ADR-0059: "Ask before acting" is the default; picking an option that skips permissions only opens the confirmation below,
                  so the control shows the confirmed choice. Once confirmed, the control and its note are red (ADR-0102). */}
              {skips.length > 0 && <div><SegmentedControl label={t('start-how-it-acts')} value={launch.permission} risk={risky} disabled={busy}
                onChange={(id) => setLaunch((s) => pickPermission(s, id))}
                items={[{ value: ASK, label: t('option-ask') }, ...skips.map((o) => ({ value: o.id, label: labelOf(o, t), icon: <Icon name="bolt" className="size-3.5" /> }))]} /></div>}
              {/* A container, so the callout puts its answers under the text in a narrow window (its own container query). */}
              {pendingOption && <div role="alertdialog" aria-label={t('session-confirm-title')} className="@container">
                <Callout tone="danger" icon="bolt" title={t('session-confirm-title')}
                  actions={<>
                    <Button variant="danger" size="sm" onClick={() => setLaunch(confirmSkip)}>{t('start-confirm-skip-yes')}</Button>
                    <Button variant="primary" size="sm" autoFocus onClick={() => setLaunch(keepAsking)}>{t('session-confirm-no')}</Button>
                  </>}>
                  <span className="text-caption text-muted">{t('start-confirm-skip-body', { name: labelOf(pendingOption, t) })}</span>
                </Callout>
              </div>}
              {risky && !pendingOption && <p data-start-risk="" className="m-0 flex items-center gap-1.5 text-caption font-medium text-danger"><Icon name="bolt" className="size-3.5" />{t('start-does-not-ask-note')}</p>}
              {plain.map((o) => <label key={o.id} className="flex items-center gap-2 text-body"><input type="checkbox" checked={launch.harmless.includes(o.id)} disabled={busy} onChange={e => { const on = e.target.checked; setLaunch((s) => toggleHarmless(s, o.id, on)); }} />{labelOf(o, t)}</label>)}
            </div>}
          </>}
          {sending.length > 0 && !error && <p role="status" className="m-0 text-caption text-muted">{t('start-settings-summary', { kind: kindName(kind), settings: sending.map(([id, value]) => `${settingName(takes.find((x) => x.id === id) as LaunchSettingChoice, t)}: ${value}`).join(', ') })}</p>}
          {error && <p role="alert" className="m-0 text-danger">{error === 'approval-not-trusted' && trust ? (binary
            // A member gets the file name only (#598): no folder to name, so the sentence asks for the folder where that file is.
            ? isBareName(binary)
              ? t('trust-hint-file', { machine: machineLabel, file: binary })
              : t('trust-hint-binary', { machine: machineLabel, where: shortBinary(binary, machineOs) })
            : t('trust-hint', { machine: machineLabel })) : t(...refusalWords(error, chosenKind?.program))}</p>}
          {error === 'approval-not-trusted' && trust && binary && shortBinary(binary, machineOs) !== binary && (wholePath
            ? <code className="break-all font-mono text-caption text-muted">{binary}</code>
            : <button type="button" className="justify-self-start text-caption text-muted underline" onClick={() => setWholePath(true)}>{t('trust-show-path')}</button>)}
          {error === 'approval-not-trusted' && askApproval?.can(node) && <AskApproval ask={askApproval.ask} />}
          {error === 'approval-not-trusted' && trust && <TrustCommand command={trustCommand(binary, machineOs)} />}
        </div>}
        <div className="flex flex-wrap items-center justify-end gap-x-2 gap-y-2.5 border-t border-line px-5 py-3">
          {info && startable && <div className="me-auto min-w-0 flex-1 basis-full sm:basis-56"><AppearsAs kind={kind} /></div>}
          <Button disabled={busy} onClick={onClose}>{t('start-cancel')}</Button>
          {info && <Button variant="primary" disabled={busy || !startable || otherProject || !!launch.pending || (!!info.previousProjectId && !acceptIdentityChange)} onClick={() => void go()}>{t('start-go')}</Button>}
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
