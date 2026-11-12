import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { CommandPalette, type PaletteItem } from './components/CommandPalette';
import { demoAgents, demoMachines, demoProjects } from './fixtures';
import { chooseLanguage, getLanguage, subscribeLanguage, t } from './i18n';
import { nextNeedingMe, type AgentTileModel } from './model';
import { connectLive, type CommandBody, type Live, type LinkState } from './net/live';
import { GrantDialog } from './components/GrantDialog';
import { grantFor, signGrant, type GrantAsk } from './net/grant';
import type { TerminalSource } from './terminal/source';
import { signDirectory, type DirectoryInfo } from './net/directory';
import { StartAgent, type KindChoice } from './components/StartAgent';
import { approvalList, boards, costByProject, projectChoices, recentMessages, machines, machinesOnline, tiles, type ApprovalModel, type View } from './net/view';
import { freshApprovals, titleWithCount } from './net/alert';
import { indexedDbStore, signApproval, thisDevice, type Device } from './net/approval';
import { addPasskey, passkeysSupported, signInWithPasskey } from './net/passkey';
import { summarize, type UsageSummary } from './net/usage';
import { forgetToken, hubConnection, readCode, signIn } from './net/session';
import { Console } from './components/Console';
import type { ChatSource } from './net/chat';
import { commitsOf, type CommitItem } from './net/history';
import type { WorkspaceFiles } from './net/files';
import { demoSource } from './terminal/source';
import { AuditPage, DevicePage, Focus, Gallery, MachinesPage, SignInPage } from './screens';
import { loadAudit } from './net/audit';

function useHash(): [string, (h: string) => void] {
  const [hash, setHash] = useState(window.location.hash);
  useEffect(() => {
    const on = () => setHash(window.location.hash);
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  return [hash, (h) => { window.location.hash = h; }];
}

export function App() {
  const language = useSyncExternalStore(subscribeLanguage, getLanguage);
  const [hash, go] = useHash();
  const [palette, setPalette] = useState(false);
  // With a hub token the fleet comes from the hub; without one (development) the demo data stands in.
  const [hub, setHub] = useState(hubConnection);
  const [signInNotice, setSignInNotice] = useState<string | undefined>();
  const trySignIn = useCallback(async (code: string) => {
    const r = await signIn(code);
    if (r.ok) { setSignInNotice(undefined); setHub(hubConnection()); return; }
    setSignInNotice(r.reason === 'refused' ? 'signin-refused' : r.reason === 'too-many' ? 'signin-too-many' : 'signin-failed');
  }, []);
  const trySignInPasskey = useCallback(async () => {
    const r = await signInWithPasskey();
    if (r.ok) { setSignInNotice(undefined); setHub(hubConnection()); return; }
    if (r.reason !== 'cancelled') setSignInNotice(r.reason === 'too-many' ? 'signin-too-many' : 'signin-passkey-failed');
  }, []);
  // A code in the address (`#code=...`) signs in at once; it is removed from the address either way.
  useEffect(() => {
    const { code, rest } = readCode(window.location.hash);
    if (code === undefined) return;
    window.history.replaceState(null, '', window.location.pathname + window.location.search + rest);
    void trySignIn(code);
  }, [trySignIn]);
  const [live, setLive] = useState<View | undefined>();
  const [link, setLink] = useState<LinkState>('connecting');
  const [api, setApi] = useState<Live | undefined>();
  // This browser's own key: what the node checks an approval against (ADR-0028).
  const [device, setDevice] = useState<Device | undefined>();
  const [usage, setUsage] = useState<UsageSummary | undefined>();
  const [notice, setNotice] = useState<string | undefined>();
  const [starting, setStarting] = useState<{ node?: string } | false>(false);
  // A machine refused a command until the owner allows it (ADR-0041): what was asked, shown as a dialog.
  const [grantAsk, setGrantAsk] = useState<{ node: string; body: CommandBody; ask: GrantAsk; busy: boolean; failed: boolean } | undefined>();
  const loadKinds = useCallback(async (node: string): Promise<KindChoice[]> => {
    const ok = await api!.command(node, { agentKinds: {} });
    return (ok.agentKinds?.kinds ?? []).map((k) => ({ kind: k.kind ?? '', name: k.displayName || (k.kind ?? ''), installed: !!k.installed }));
  }, [api]);
  const inspectDirectory = useCallback(async (node: string, path: string, createProjectId: boolean): Promise<DirectoryInfo> => {
    if (!api || !device) throw { code: 'approval.signature_invalid' };
    const authorization = await signDirectory(device, node, 'inspect', path, '', '', createProjectId);
    const ok = await api.command(node, { directoryInspect: { path, createProjectId, authorization } });
    const i = ok.directoryInfo;
    if (!i) throw { code: 'protocol.capability_missing' };
    return { path: i.path ?? '', projectId: i.projectId ?? '', gitRoot: i.gitRoot ?? '', branch: i.branch ?? '', portable: !!i.portable, previousProjectId: i.previousProjectId ?? '' };
  }, [api, device]);
  const startAgent = useCallback(async (node: string, info: DirectoryInfo, kind: string, separateWorktree: boolean, acceptIdentityChange: boolean): Promise<string | undefined> => {
    try {
      if (!api || !device) return 'approval-not-trusted';
      const authorization = await signDirectory(device, node, 'launch', info.path, kind, info.projectId, separateWorktree, acceptIdentityChange);
      const ok = await api.command(node, { agentLaunch: { path: info.path, projectId: info.projectId, kind, separateWorktree, acceptIdentityChange, authorization } });
      const id = ok.agentLaunch?.agentId;
      setStarting(false);
      if (id) window.location.hash = `#/agent/${encodeURIComponent(`${node}/${id}`)}`;
      return undefined;
    } catch (e) {
      const code = (e as { code?: string }).code;
      return code === 'agent.mode_unsupported' ? 'start-directory-agent-unsupported' : code === 'approval.signature_invalid' ? 'approval-not-trusted' : code === 'protocol.capability_missing' ? 'start-directory-unsupported' : code === 'agent.kind_unknown' ? 'start-kind-unknown' : code === 'node.resource_limit' ? 'start-limit' : 'start-failed';
    }
  }, [api, device]);
  useEffect(() => {
    const store = indexedDbStore();
    if (store) void thisDevice(store).then(setDevice).catch(() => setDevice(undefined));
  }, []);
  const resolve = useCallback(async (a: ApprovalModel, approved: boolean) => {
    if (!api || !device) { setNotice('approval-failed'); return; }
    setNotice(undefined);
    try {
      const signature = await signApproval(device, a.node, a.localId, approved);
      await api.command(a.node, { approvalResolve: { approvalId: a.localId, approved, signerDeviceId: device.id, signature } });
    } catch (e) {
      const code = (e as { code?: string }).code;
      setNotice(code === 'approval.signature_invalid' ? 'approval-not-trusted' : 'approval-failed');
    }
  }, [api, device]);
  // The owner allowed it: sign the grant with this device's key, send it to the machine, then repeat what was refused.
  const allowGrant = useCallback(async () => {
    const g = grantAsk;
    if (!g || !api || !device) { setGrantAsk((c) => c && { ...c, failed: true }); return; }
    setGrantAsk({ ...g, busy: true, failed: false });
    try {
      const s = await signGrant(device, g.node, g.ask, Date.now());
      await api.command(g.node, { grantCreate: s });
      setGrantAsk(undefined);
      if (!('terminalResize' in g.body)) await api.command(g.node, g.body).catch(() => {});
    } catch {
      setGrantAsk({ ...g, busy: false, failed: true });
    }
  }, [api, device, grantAsk]);
  // Approvals waiting while the tab is hidden: a count in the title, and a notification if the user allowed it.
  const seenApprovals = useRef<Set<string>>(new Set());
  const baseTitle = useRef(document.title);
  useEffect(() => {
    if (!live) return;
    const list = approvalList(live);
    document.title = titleWithCount(baseTitle.current, list.length);
    const fresh = freshApprovals(seenApprovals.current, list);
    seenApprovals.current = new Set(list.map((a) => a.id));
    if (document.visibilityState === 'visible' || typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
    for (const a of fresh) {
      try { new Notification(t('approval-title', { name: a.agentName }), { body: a.action, tag: a.id }); } catch { /* not allowed here */ }
    }
  }, [live]);
  // Browsers only ask for notification permission after a user gesture: ask at the first click once signed in.
  useEffect(() => {
    if (!hub || typeof Notification === 'undefined' || Notification.permission !== 'default') return;
    const ask = () => { try { void Notification.requestPermission(); } catch { /* unsupported */ } };
    window.addEventListener('pointerdown', ask, { once: true });
    return () => window.removeEventListener('pointerdown', ask);
  }, [hub]);
  // What the agents cost so far, estimated by the hub from its price table; asked for again every half minute.
  useEffect(() => {
    if (!api || !live) return;
    let stop = false;
    const ask = () => { api.command('', { usageReport: {} }).then((ok) => { if (!stop && ok.usageReport) setUsage(summarize(ok.usageReport)); }).catch(() => {}); };
    ask();
    const timer = setInterval(ask, 30_000);
    return () => { stop = true; clearInterval(timer); };
  }, [api, live !== undefined]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!hub) return;
    const c = connectLive({ url: hub.url, token: hub.token, onView: (v) => setLive({ ...v }), onState: setLink,
      onRejected: () => { forgetToken(); setLive(undefined); setHub(undefined); },
      onGrantRequired: (node, body) => {
        const ask = grantFor(body as Record<string, unknown>);
        if (ask) setGrantAsk((cur) => cur ?? { node, body, ask, busy: false, failed: false });
      } });
    setApi(c);
    return () => { c.close(); setApi(undefined); };
  }, [hub]);
  const agents = useMemo(() => (hub ? (live ? tiles(live) : []) : demoAgents), [hub, live]);
  const current = hash.startsWith('#/agent/') ? decodeURIComponent(hash.slice('#/agent/'.length)) : undefined;

  // An agent's id here is the hub-wide key `<node>/<local id>`; its terminal is `t-<local id>` on that node.
  const terminal: TerminalSource | undefined = useMemo(() => {
    const at = current?.indexOf('/') ?? -1;
    if (!api || !current || at < 1) return undefined;
    return api.terminal(current.slice(0, at), `t-${current.slice(at + 1)}`);
  }, [api, current]);

  // One source per agent, kept for as long as the agent is on the wall, so a terminal keeps its connection when others change.
  const sources = useRef(new Map<string, TerminalSource>());
  useEffect(() => { sources.current.clear(); }, [api]);
  const terminalFor = useCallback((id: string): TerminalSource => {
    let s = sources.current.get(id);
    if (!s) {
      const at = id.indexOf('/');
      s = api && at > 0 ? api.terminal(id.slice(0, at), `t-${id.slice(at + 1)}`) : demoSource(agents.find((a) => a.id === id)?.preview ?? '');
      sources.current.set(id, s);
    }
    return s;
  }, [api, agents]);

  const loadDiff = useCallback(async (a: AgentTileModel): Promise<string> => {
    if (!api || !a.workspaceRef) return '';
    const ok = await api.command(a.workspaceRef.node, a.workspaceRef.directory ? { workspaceQuery: { workspaceId: a.workspaceRef.id, operation: 'diff' } } : { gitDiff: { worktreeId: a.workspaceRef.id, path: '' } });
    return ok.gitDiff?.diff ?? '';
  }, [api]);

  // File access per workspace, kept stable so an open file browser does not reload when the fleet updates.
  const fileAccess = useRef(new Map<string, WorkspaceFiles>());
  useEffect(() => { fileAccess.current.clear(); }, [api]);
  const filesOf = useCallback((a: AgentTileModel): WorkspaceFiles | undefined => {
    const w = a.workspaceRef;
    if (!api || !w) return undefined;
    const key = `${w.node}/${w.id}`;
    let f = fileAccess.current.get(key);
    if (!f) {
      f = {
        list: async (path) => {
          const ok = await api.command(w.node, w.directory ? { workspaceQuery: { workspaceId: w.id, operation: 'list', path } } : { filesList: { worktreeId: w.id, path } });
          return (ok.filesList?.entries ?? []).map((e) => ({ name: e.name ?? '', isDir: !!e.isDir, size: Number(e.sizeBytes ?? 0) }));
        },
        read: async (path) => (await api.command(w.node, w.directory ? { workspaceQuery: { workspaceId: w.id, operation: 'read', path } } : { filesRead: { worktreeId: w.id, path } })).filesRead?.data ?? new Uint8Array(),
      };
      fileAccess.current.set(key, f);
    }
    return f;
  }, [api]);

  const sendPrompt = useCallback(async (a: AgentTileModel, text: string): Promise<boolean> => {
    if (!api || !a.workspaceRef) return false;
    try {
      await api.command(a.workspaceRef.node, { agentPrompt: { agentId: a.id.slice(a.id.indexOf('/') + 1), text } });
      return true;
    } catch { return false; }
  }, [api]);

  const loadHistory = useCallback(async (a: AgentTileModel): Promise<CommitItem[]> => {
    if (!api || !a.workspaceRef) return [];
    const ok = await api.command(a.workspaceRef.node, a.workspaceRef.directory ? { workspaceQuery: { workspaceId: a.workspaceRef.id, operation: 'history' } } : { gitLog: { worktreeId: a.workspaceRef.id, limit: 0 } });
    return commitsOf(ok.gitLog?.commits);
  }, [api]);

  // One conversation source per agent, kept stable so the transcript does not reload when the fleet updates.
  const chats = useRef(new Map<string, ChatSource>());
  useEffect(() => { chats.current.clear(); }, [api]);
  const chatOf = useCallback((a: AgentTileModel): ChatSource | undefined => {
    if (!api || !a.workspaceRef) return undefined;
    let c = chats.current.get(a.id);
    if (!c) {
      c = api.chat(a.workspaceRef.node, a.id.slice(a.id.indexOf('/') + 1));
      chats.current.set(a.id, c);
    }
    return c;
  }, [api]);

  const selectedAgent = agents.find(a => a.id === current);

  const open = useCallback((id: string) => go(`#/agent/${encodeURIComponent(id)}`), [go]);
  const next = useCallback(() => {
    const n = nextNeedingMe(agents, current);
    if (n) open(n.id);
  }, [agents, current, open]);

  const items: PaletteItem[] = useMemo(() => [
    { id: 'next', label: t('palette-next-attention'), run: next },
    { id: 'home', label: t('palette-go-home'), run: () => go('#/') },
    ...(hub ? [{ id: 'start', label: t('action-start-agent'), run: () => { go('#/'); setStarting({}); } }] : []),
    { id: 'machines', label: t('palette-go-machines'), run: () => go('#/machines') },
    ...(hub ? [{ id: 'device', label: t('device-link'), run: () => go('#/device') }, { id: 'audit', label: t('audit-link'), run: () => go('#/audit') }] : []),
    // The language, here too: on a phone the title bar has no room for its picker.
    ...(['system', 'en', 'tr'] as const).map((l) => ({ id: `language-${l}`, label: t('palette-language', { name: t(`language-${l}`) }), run: () => { chooseLanguage(l); } })),
    ...agents.map((a) => ({ id: `agent-${a.id}`, label: t('palette-open-agent', { name: a.name }), run: () => open(a.id) })),
  ], [agents, go, hub, next, open, language]);

  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName);
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); setPalette((p) => !p); }
      else if (!typing && !e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === 'n') next();
    };
    window.addEventListener('keydown', on);
    return () => window.removeEventListener('keydown', on);
  }, [next]);

  // Served by the hub (a production build) and not signed in: ask for the code. Development keeps the demo data.
  if (!hub && !import.meta.env.DEV && hash !== '#/gallery' && hash !== '#/demo') {
    return <SignInPage onSubmit={trySignIn} onPasskey={passkeysSupported() ? trySignInPasskey : undefined} notice={signInNotice ? t(signInNotice) : undefined} />;
  }

  return (
    <>
      {hash === '#/gallery' ? <Gallery /> : hash === '#/machines' ? <MachinesPage machines={live ? machines(live) : []} usage={usage} projects={live && usage ? costByProject(live, usage.perAgent) : []} onBack={() => go('#/')} /> : hash === '#/audit' && hub ? <AuditPage load={(who) => loadAudit(hub.token, who)} onBack={() => go('#/')} /> : hash === '#/device' ? <DevicePage device={device} onBack={() => go('#/')} onAddPasskey={hub && passkeysSupported() ? async () => (await addPasskey(hub.token)).ok : undefined} /> : current !== undefined
        ? <Focus agent={selectedAgent} chatSource={selectedAgent?.terminal === false ? chatOf(selectedAgent) : undefined} sendPrompt={hub && api ? sendPrompt : undefined} costMicroUsd={current ? usage?.perAgent.get(current) : undefined} source={terminal} onBack={() => go('#/')} />
        : <Console agents={agents} machines={live ? machines(live) : []} usage={usage} projects={live && usage ? costByProject(live, usage.perAgent) : []}
          approvals={live ? approvalList(live) : []} messages={live ? recentMessages(live) : []} boards={live ? boards(live) : []} terminalFor={terminalFor} chatOf={hub && api ? chatOf : undefined} sendPrompt={hub && api ? sendPrompt : undefined} loadDiff={hub && api ? loadDiff : undefined} loadHistory={hub && api ? loadHistory : undefined} filesOf={hub && api ? filesOf : undefined} onOpen={open}
          onResolve={resolve} notice={notice ? t(notice) : undefined} link={hub ? link : 'live'}
          onStart={hub && api && live ? (node) => setStarting({ node }) : (!hub && import.meta.env.DEV ? (node) => setStarting({ node }) : undefined)} onCommand={() => setPalette(true)}
          start={!hub && import.meta.env.DEV && starting ? <StartAgent initialNode={starting.node} projects={demoProjects} machines={demoMachines}
            loadKinds={async () => [{ kind: 'claude-code', name: 'Claude Code', installed: true }, { kind: 'codex', name: 'Codex', installed: true }, { kind: 'shell', name: 'Shell', installed: true }, { kind: 'gemini', name: 'Gemini CLI', installed: false }]}
            inspect={async (_n, path) => ({ path, projectId: 'demo', gitRoot: path, branch: 'main', portable: false })} onStart={async () => { setStarting(false); return undefined; }} onClose={() => setStarting(false)} /> : starting && live ? <StartAgent initialNode={starting ? starting.node : undefined} projects={projectChoices(live)} machines={machines(live)} inspect={inspectDirectory} loadKinds={loadKinds} onStart={startAgent} onClose={() => setStarting(false)} /> : undefined} />}
      {grantAsk && <GrantDialog machine={live?.nodes.get(grantAsk.node)?.hostname || grantAsk.node} kind={grantAsk.ask.kind} busy={grantAsk.busy} failed={grantAsk.failed} onAllow={() => void allowGrant()} onClose={() => setGrantAsk(undefined)} />}
      {palette && <CommandPalette items={items} onClose={() => setPalette(false)} />}
    </>
  );
}
