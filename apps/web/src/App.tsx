import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { approvalName } from './net/approvalText';
import { CommandPalette, type PaletteItem } from './components/CommandPalette';
import { demoAgents, demoApprovals, demoList, demoMachines, demoProjects, demoQuota } from './fixtures';
import { QuotaProvider } from './components/Quota';
import { demoDiff, demoFiles } from './demoWorkspace';
import { chooseLanguage, getLanguage, subscribeLanguage, t } from './i18n';
import { nextNeedingMe, type AgentTileModel, type ResumeAction } from './model';
import type { DiffSource } from './net/diff';
import { usePairing } from './net/usePairing';
import { mayChange, ownsMachines, type PersonRole } from './net/role';
import { PairRequest } from './components/PairRequest';
import { connectLive, type CommandBody, type Live, type LinkState } from './net/live';
import { GrantDialog } from './components/GrantDialog';
import { signGrant } from './net/grant';
import { useGrantFlow } from './net/useGrantFlow';
import type { TerminalSource } from './terminal/source';
import { signDirectory, type DirectoryInfo, type DirectoryListing } from './net/directory';
import { StartAgent, type KindChoice } from './components/StartAgent';
import { quotaObs, approvalList, boards, costByProject, projectChoices, recentMessages, machines, machinesOnline, shown, tiles, type ApprovalModel, type View } from './net/view';
import { approvalFailureKey, indexedDbStore, signApproval, signStart, thisDevice, type Device } from './net/approval';
import { answerFields, answerPlan } from './net/always';
import type { StartChoice } from './components/StartApproval';
import { forgetSkipAnswer, forgetSkipCapable, isSkipCapable, recordSkipAnswer, signSkip, SKIP_ACTION } from './net/skipApproval';
import type { HelperGrant } from './components/HelperGrants';
import { addPasskey, passkeysSupported, signInWithPasskey } from './net/passkey';
import { endSession, forgetToken, hubConnection, readCode, setPassword, signIn, signInFromLink, NO_PASSWORD } from './net/session';
import { useSignInInfo } from './net/useSignInInfo';
import { idleMinutes, setIdleMinutes, watchIdle, type IdleMinutes } from './net/idle';
import { ScreenPasswordSettings } from './components/Locks';
import { Console } from './components/Console';
import type { ChatSource } from './net/chat';
import { commitsOf, type CommitItem } from './net/history';
import type { WorkspaceFiles } from './net/files';
import { demoSource } from './terminal/source';
import { AuditPage, DevicePage, Focus, Gallery, MachinesPage, SettingsDemo, SignInPage, type MyDevice } from './screens';
import { browserName, canBeApprovedOn, deviceLabel, fingerprint, fromHex, registrationVerifies, signAsk, signEndorsement, signRegistration, signRevocation, trustedBy, waitingForApproval, type DeviceInfo } from './net/devices';
import { DeviceRequest } from './components/DeviceRequest';
import { num } from './net/view';
import { loadAudit } from './net/audit';
import { agentTarget } from './components/sessionChoice';
import { ChatActionsContext, type ChatActions, type PromptResult } from './components/chatActions';
import { CloseAgentContext } from './components/CloseAgent';
import { closeAsk } from './net/closeAgent';
import { canStartFor, forgetGiveSupport, forgetOffline, givableTasks, giveAsk, giveTask, useGiveAsk, useGiveSupport } from './net/give';
import { GiveContext, GiveDialog, type GiveApi } from './components/GiveTask';
import { plain } from './components/boardColumns';
import { failure, type ActionResult } from './net/refusal';
import { refusalCode, startRefusalKey } from './net/startRefusal';
import { useUsageReport } from './net/useUsage';
import { usableId } from './components/launchChoice';
import { launchSettingsField } from './components/launchSettingsModel';
import { rememberStarted } from './components/launchMemory';
import { PhoneAgent, PhoneHome, PhoneTheme, usePhone } from './phone/Phone';
import { approvalRedirect, phoneScreen } from './phone/nav';
import { AttentionLayer } from './components/AttentionLayer';
import { AgentAlerts } from './components/AgentAlerts';
import { useOnScreen } from './net/onScreen';
import { AnswerHoldsProvider } from './components/useAnswerGuard';
import { attentionList } from './net/fleet';
import { requestPanel, requestProject, requestTool } from './layout/requests';
import { TOOL_PANELS } from './net/paneTools';
import { decodeHash, isMachinesHash, isSettingsHash, machineOfHash, settingsSectionOf, paletteExtras, projectsOf } from './net/paletteExtras';
import { themeStore } from './themes/store';
import { themeName } from './themes/messages';
import { COMMANDS, commandById } from './keys/commands';
import { type Handler, handlerOf, listenForKeys, runCommand, useCommands, useCommandsVersion } from './keys/registry';
import { keyStore } from './keys/store';
import { IS_MAC } from './keys/strokes';
import { ariaKey, commandLabel, shownKey, useKeymap } from './keys/view';
import { PluginsDialog, SafeModeNote } from './plugins/PluginsDialog';
import { SAFE, safeAddress } from './plugins/safe';

// ADR-0059: why the machine refused the chosen launch option, as a message id (its `policy` parameter, a stable value).
function optionForbidden(e: unknown): string {
  const policy = (e as { params?: Record<string, string> }).params?.policy;
  return policy === 'confined_only' ? 'start-option-confined-only' : 'start-option-forbidden';
}

function useHash(): [string, (h: string) => void] {
  const [hash, setHash] = useState(window.location.hash);
  useEffect(() => {
    const on = () => setHash(window.location.hash);
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  return [hash, (h) => { window.location.hash = h; }];
}

// The icon of each tool in the command palette (the same ones the tool panes use).

export function App() {
  const language = useSyncExternalStore(subscribeLanguage, getLanguage);
  const [hash, go] = useHash();
  // ADR-0067 rung 2: a phone gets one column (the list, then one agent); a tablet and a desktop keep the window.
  const phone = usePhone();
  const [palette, setPalette] = useState(false);
  // Plugins loaded from a local file, for development (ADR-0114): the palette opens the list, the loading and the install screen.
  const [pluginsOpen, setPluginsOpen] = useState(false);
  // With a hub token the fleet comes from the hub; without one (development) the demo data stands in.
  const [hub, setHub] = useState(hubConnection);
  const [signInNotice, setSignInNotice] = useState<string | undefined>();
  // Whether the hub asks this browser for the screen password (ADR-0067), and whether one is set.
  const [info, refreshInfo] = useSignInInfo(hub);
  const trySignIn = useCallback(async (code: string, password?: string) => {
    const r = await signIn(code, password);
    if (r.ok) { setSignInNotice(undefined); setHub(hubConnection()); return; }
    if (r.reason === 'busy') { setSignInNotice('password-busy'); return; }
    setSignInNotice(r.reason === 'refused' ? (password ? 'signin-refused-password' : 'signin-refused') : r.reason === 'too-many' ? 'signin-too-many' : 'signin-failed');
  }, []);
  // A code from a `#code=` link that still needs the screen password: kept in the form, not submitted.
  const [linkCode, setLinkCode] = useState<string | undefined>();
  const [idle, setIdle] = useState<IdleMinutes>(() => idleMinutes());
  const trySignInPasskey = useCallback(async () => {
    const r = await signInWithPasskey();
    if (r.ok) { setSignInNotice(undefined); setHub(hubConnection()); return; }
    if (r.reason === 'busy') { setSignInNotice('password-busy'); return; }
    if (r.reason !== 'cancelled') setSignInNotice(r.reason === 'too-many' ? 'signin-too-many' : 'signin-passkey-failed');
  }, []);
  // A code in the address (`#code=...`) is removed from the address at once. It signs in at once unless the hub wants the screen
  // password from here: then it waits in the form for the password.
  useEffect(() => {
    const { code, rest } = readCode(window.location.hash);
    if (code === undefined) return;
    window.history.replaceState(null, '', window.location.pathname + window.location.search + rest);
    void signInFromLink(code, async () => (await refreshInfo()) ?? NO_PASSWORD, (c) => { void trySignIn(c); }, setLinkCode);
  }, [trySignIn, refreshInfo]);
  const [live, setLive] = useState<View | undefined>();
  const [link, setLink] = useState<LinkState>('connecting');
  const [api, setApi] = useState<Live | undefined>();
  // ADR-0106: the role of the person this window signed in as, as the hub's first frame said (`unspecified` from an older hub). A viewer's
  // window offers nothing that changes something: the hub would refuse it, and write the refusal down in the owner's Activity.
  const [role, setRole] = useState<PersonRole>('unspecified');
  const changes = mayChange(role);
  // ADR-0065: machines asking to join (owner only, memory only).
  const pairing = usePairing(api, link, role);
  // This browser's own key: what the node checks an approval against (ADR-0028).
  const [device, setDevice] = useState<Device | undefined>();
  // ADR-0061: this person's registered devices as the hub lists them, their fingerprints, and the requests put off for now.
  const [mine, setMine] = useState<DeviceInfo[]>([]);
  const [prints, setPrints] = useState<Map<string, string>>(new Map());
  const [later, setLater] = useState<Set<string>>(new Set());
  const [proven, setProven] = useState<Set<string>>(new Set());
  const [notice, setNotice] = useState<string | undefined>();
  const [starting, setStarting] = useState<{ node?: string; kind?: string } | false>(false);
  // A terminal started from "New terminal" opens as a Terminal pane of the window instead of taking the whole window.
  const startedAsTerminal = useRef(false);
  const openStart = useCallback((node?: string, terminal = false) => { startedAsTerminal.current = terminal; setStarting({ ...(node ? { node } : {}), ...(terminal ? { kind: 'shell' } : {}) }); }, []);
  // A machine refused a command until the owner allows it (ADR-0041): what was asked, shown as a dialog.
  const loadKinds = useCallback(async (node: string): Promise<KindChoice[]> => {
    const ok = await api!.command(node, { agentKinds: {} });
    return (ok.agentKinds?.kinds ?? []).map((k) => ({
      kind: k.kind ?? '', name: k.displayName || (k.kind ?? ''), installed: !!k.installed, level: Number(k.level ?? 0),
      // ADR-0069: the program the machine looks for in its PATH (empty from an older machine).
      program: k.program ?? '',
      // ADR-0059: the kind's launch options (the machine keeps their arguments).
      // ADR-0066: the kind's launch settings (model, effort) and what each takes.
      settings: (k.settings ?? []).map((st) => ({ id: st.id ?? '', label: st.label ?? '', values: st.values ?? [], defaultValue: st.defaultValue ?? '' })),
      options: (k.launchOptions ?? []).filter((o) => usableId(o.id ?? '')).map((o) => ({ id: o.id ?? '', label: o.label ?? '', skipsPermissions: !!o.skipsPermissions })),
    }));
  }, [api]);
  const inspectDirectory = useCallback(async (node: string, path: string, createProjectId: boolean): Promise<DirectoryInfo> => {
    if (!api || !device) throw { code: 'approval.signature_invalid' };
    const authorization = await signDirectory(device, node, 'inspect', path, '', '', createProjectId);
    const ok = await api.command(node, { directoryInspect: { path, createProjectId, authorization } });
    const i = ok.directoryInfo;
    if (!i) throw { code: 'protocol.capability_missing' };
    return { path: i.path ?? '', projectId: i.projectId ?? '', gitRoot: i.gitRoot ?? '', branch: i.branch ?? '', portable: !!i.portable, previousProjectId: i.previousProjectId ?? '' };
  }, [api, device]);
  const listDirectory = useCallback(async (node: string, path: string): Promise<DirectoryListing> => {
    if (!api || !device) throw { code: 'approval.signature_invalid' };
    const authorization = await signDirectory(device, node, 'list', path);
    const ok = await api.command(node, { directoryList: { path, authorization } });
    const d = ok.directoryEntries;
    if (!d) throw { code: 'protocol.capability_missing' };
    return { path: d.path ?? '', parent: d.parent ?? '', entries: (d.entries ?? []).map((e) => ({ name: e.name ?? '', isGit: !!e.isGit })), truncated: !!d.truncated };
  }, [api, device]);
  // The ordinary start command (ADR-0052, ADR-0059, ADR-0066), signed by this device: the new agent's id on its machine, or the refusal's
  // locale key. "Start an agent" and "Start a new agent for this task" (ADR-0095 step 3) both start through this and nothing else.
  const launchAgent = useCallback(async (node: string, info: DirectoryInfo, kind: string, separateWorktree: boolean, acceptIdentityChange: boolean, launch: { ids: string[]; confirmed: string[]; settings: [string, string][] }): Promise<{ agentId: string } | { failure: string }> => {
    try {
      if (!api || !device) return { failure: 'approval-not-trusted' };
      // ADR-0059: the device signs the chosen launch options too, so the hub can neither add nor remove one.
      const authorization = await signDirectory(device, node, 'launch', info.path, kind, info.projectId, separateWorktree, acceptIdentityChange, launch.ids, launch.confirmed, launch.settings);
      const ok = await api.command(node, { agentLaunch: { path: info.path, projectId: info.projectId, kind, separateWorktree, acceptIdentityChange, authorization, launchOptions: launch.ids, confirmedLaunchOptions: launch.confirmed, ...launchSettingsField(launch.settings) } });
      const id = ok.agentLaunch?.agentId ?? '';
      if (id) rememberStarted(`${node}/${id}`, launch.settings);
      return { agentId: id };
    } catch (e) {
      const code = refusalCode(e, 'AgentLaunch');
      if (code === 'agent.option_forbidden') return { failure: optionForbidden(e) };
      if (code === 'agent.option_unconfirmed') return { failure: 'start-option-unconfirmed' };
      if (code === 'agent.option_invalid') return { failure: 'start-option-invalid' };
      if (code === 'agent.setting_unknown' || code === 'agent.setting_invalid') return { failure: 'start-setting-refused' };
      if (code === 'protocol.capability_missing' && launch.settings.length > 0) return { failure: 'start-settings-unsupported' };
      if (code === 'protocol.capability_missing' && launch.ids.length > 0) return { failure: 'start-options-unsupported' };
      return { failure: startRefusalKey(code) };
    }
  }, [api, device]);
  const startAgent = useCallback(async (node: string, info: DirectoryInfo, kind: string, separateWorktree: boolean, acceptIdentityChange: boolean, launch: { ids: string[]; confirmed: string[]; settings: [string, string][] }): Promise<string | undefined> => {
    const r = await launchAgent(node, info, kind, separateWorktree, acceptIdentityChange, launch);
    if ('failure' in r) return r.failure;
    const id = r.agentId;
    setStarting(false);
    if (id && startedAsTerminal.current && kind === 'shell') requestTool({ panel: 'terminal', agent: `${node}/${id}`, wait: true });
    else if (id) window.location.hash = `#/agent/${encodeURIComponent(`${node}/${id}`)}`;
    return undefined;
  }, [launchAgent]);
  useEffect(() => {
    const store = indexedDbStore();
    if (store) void thisDevice(store).then(setDevice).catch(() => setDevice(undefined));
  }, []);
  // ADR-0060: brings back an interrupted agent under the same id, with its own session or (`newSession`) a new one in the same folder.
  // Signed by this browser for exactly that agent and choice; the answer is undefined when it started, else the error code.
  const resumeAgent = useCallback(async (agent: AgentTileModel, action: ResumeAction): Promise<string | undefined> => {
    if (!api || !device || !agent.machineId) return 'approval.signature_invalid';
    const local = agent.id.slice(agent.id.indexOf('/') + 1);
    const newSession = action === 'again';
    try {
      // Its process from before the restart: ended by the node (exactly that process), like any stop.
      if (action === 'stop') { await api.command(agent.machineId, { agentStop: { agentId: local, stopHelpers: false } }); return undefined; }
      const authorization = await signDirectory(device, agent.machineId, 'resume', local, '', '', newSession);
      await api.command(agent.machineId, { agentResume: { agentId: local, newSession, authorization } });
      return undefined;
    } catch (e) { return refusalCode(e, 'AgentResume') ?? 'internal'; }
  }, [api, device]);
  // Ends a running agent (`AgentStop`, as `agora agent stop` does) for exactly the agent it is given; undefined when it was sent, else the error
  // code. The node may ask for a grant first (the dialog then repeats this command once it is allowed); the agent then lands in the closed list.
  const closeAgent = useCallback(async (agent: AgentTileModel): Promise<string | undefined> => {
    if (!api || !agent.machineId) return 'hub.unreachable';
    const local = agent.id.slice(agent.id.indexOf('/') + 1);
    try { await api.command(agent.machineId, { agentStop: { agentId: local, stopHelpers: false } }); return undefined; } catch (e) { return (e as { code?: string }).code ?? 'hub.unreachable'; }
  }, [api]);
  // Asks the hub to forget a closed agent for every client (ADR-0053). The answer is undefined when it did, else the error code.
  const forgetAgent = useCallback(async (agentId: string): Promise<string | undefined> => {
    if (!api) return 'hub.unreachable';
    try { await api.command('', { agentForget: { agentId } }); return undefined; } catch (e) { return (e as { code?: string }).code ?? 'hub.unreachable'; }
  }, [api]);
  // ADR-0073: asks the hub to forget machines, one command each; the answer per machine is undefined when it did, else the code.
  const forgetMachines = useCallback(async (ids: string[], refuseReturn: boolean): Promise<Map<string, string | undefined>> => {
    const out = new Map<string, string | undefined>();
    for (const nodeId of ids) {
      if (!api) { out.set(nodeId, 'hub.unreachable'); continue; }
      try { await api.command('', { nodeForget: { nodeId, refuseReturn } }); out.set(nodeId, undefined); } catch (e) { out.set(nodeId, (e as { code?: string }).code ?? 'hub.unreachable'); }
    }
    return out;
  }, [api]);
  // `always` (ADR-0101): the person pressed "Allow always"; sent so only for a request that offers it (`answerPlan`), never otherwise.
  // It says whether the answer went (Home's request cards re-arm at once after a failed one).
  const resolve = useCallback(async (a: ApprovalModel, approved: boolean, choice?: StartChoice, always = false): Promise<boolean> => {
    if (!api || !device) { setNotice('approval-failed'); return false; }
    setNotice(undefined);
    if (answerPlan(a, approved, always).scope === 'none') { setNotice('approval-scope-unavailable'); return false; }
    try {
      // ADR-0057: a start is decided over the request exactly as shown, with the grant chosen.
      const helperGrant = a.start && approved ? choice?.grant ?? 0 : 0;
      const helpersMayStart = !!(a.start && approved && choice?.nested);
      // ADR-0090 Amendment A: a switch to a mode that skips permission questions is signed over its own domain, only for a machine heard
      // to offer it and only in the shape the window expects; any other shape is never signed.
      if (a.skipUnknown || (a.action === SKIP_ACTION && !a.skip)) { setNotice('skip-approval-unknown'); return false; }
      if (a.skip && !isSkipCapable(a.node)) { setNotice('skip-approval-other-window'); return false; }
      // ADR-0101: an "always" answer over its own bytes with scope ALWAYS; any other over its own message, with no scope.
      const fields = await answerFields(device, a, approved, always, () => (a.skip
        ? signSkip(device, a.node, a.localId, approved, a.skip, a.detail)
        : a.start
          ? signStart(device, a.node, a.localId, approved, helperGrant, helpersMayStart, a.detail)
          : signApproval(device, a.node, a.localId, approved)));
      if (!fields) { setNotice('approval-scope-unavailable'); return false; }
      const { signature } = fields;
      // What this window answered, kept before the answer goes so a switch applied at once is still known as approved here.
      if (a.skip) recordSkipAnswer(a.id, { approved, agent: `${a.node}/${a.skip.agent}`, option: a.skip.option, value: a.skip.value });
      await api.command(a.node, { approvalResolve: { approvalId: a.localId, approved, signerDeviceId: device.id, signature, helperGrant, helpersMayStart, ...(fields.scope ? { scope: fields.scope } : {}) } });
      return true;
    } catch (e) {
      forgetSkipAnswer(a.id);
      setNotice(approvalFailureKey(refusalCode(e, 'ApprovalResolve')));
      return false;
    }
  }, [api, device]);
  // ADR-0106: a viewer's window shows every request read-only (no Allow, Deny or Allow always): the hub would refuse the answer and write
  // the refusal down in the owner's Activity.
  const answerRequest = changes ? resolve : undefined;
  // ADR-0057: an agent's live helper grants, as its machine lists them, and revoking one (no signature: ending access is always allowed).
  const helperGrants = useCallback(async (agent: AgentTileModel): Promise<HelperGrant[]> => {
    if (!api || !agent.machineId) return [];
    const local = agent.id.slice(agent.id.indexOf('/') + 1);
    const ok = await api.command(agent.machineId, { grantList: {} });
    return (ok.grantList?.grants ?? [])
      .filter((g) => g.scope === `agent:${local}` && (g.commands ?? []).includes('mesh.agent_start'))
      .map((g) => ({ id: g.grantId ?? '', kind: g.kind ?? '', folder: g.folder ?? '', remaining: g.remaining ?? 0 }));
  }, [api]);
  const revokeGrant = useCallback(async (agent: AgentTileModel, id: string) => {
    if (!api || !agent.machineId) return;
    await api.command(agent.machineId, { grantRevoke: { grantId: id } });
  }, [api]);
  // The owner allowed it: sign the grant with this device's key, send it to the machine, then repeat what was refused (net/useGrantFlow.ts).
  const grants = useGrantFlow({ command: api?.command, sign: device ? (node, ask) => signGrant(device, node, ask, Date.now()) : undefined });
  const grantAsk = grants.question;
  // ADR-0061: this browser registers its key and name with the hub on every connection (a candidate for the person's machines, never
  // trust by itself), then follows the person's devices: which wait for an approval, which machines trust which.
  const refreshDevices = useCallback(async () => {
    if (!api) return;
    try { setMine((await api.command('', { deviceList: {} })).deviceList?.devices ?? []); } catch { /* an older hub has no device list */ }
  }, [api]);
  useEffect(() => {
    if (!api || !device || !live) return;
    let stop = false;
    void signRegistration(device, browserName(), Date.now()).then((r) => api.command('', { deviceRegister: r }))
      .catch(() => undefined).then(() => { if (!stop) void refreshDevices(); });
    const timer = setInterval(() => { void refreshDevices(); }, 15_000);
    return () => { stop = true; clearInterval(timer); };
  }, [api, device, live !== undefined, refreshDevices]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    let stop = false;
    const keys: [string, Uint8Array][] = mine.map((d) => [d.deviceId ?? '', d.publicKey ?? new Uint8Array()]);
    if (device) keys.push([device.id, fromHex(device.publicKeyHex)]);
    void Promise.all(keys.map(async ([id, k]) => [id, k.length === 65 ? await fingerprint(k) : ''] as const))
      .then((e) => { if (!stop) setPrints(new Map(e)); }, () => undefined);
    void Promise.all(mine.map(async (d) => ((await registrationVerifies(d)) ? d.deviceId ?? '' : '')))
      .then((ids) => { if (!stop) setProven(new Set(ids.filter(Boolean))); }, () => undefined);
    return () => { stop = true; };
  }, [mine, device]);
  const askApproval = useCallback(async (): Promise<boolean> => {
    if (!api || !device) return false;
    try { await api.command('', { deviceAskApproval: await signAsk(device, Date.now()) }); void refreshDevices(); return true; } catch { return false; }
  }, [api, device, refreshDevices]);
  const approveDevice = useCallback(async (d: DeviceInfo, nodes: string[]): Promise<boolean> => {
    if (!api || !device || !d.deviceId || !d.publicKey) return false;
    try {
      const endorsement = await signEndorsement(device, { deviceId: d.deviceId, publicKey: d.publicKey, name: d.name ?? '' }, nodes, Date.now());
      await api.command('', { deviceEndorse: { endorsement } });
      void refreshDevices();
      return true;
    } catch { return false; }
  }, [api, device, refreshDevices]);
  const removeDevice = useCallback(async (id: string): Promise<boolean> => {
    if (!api || !device) return false;
    try { await api.command('', { deviceRevoke: { revocation: await signRevocation(device, id, Date.now()) } }); void refreshDevices(); return true; } catch { return false; }
  }, [api, device, refreshDevices]);
  const trustedOn = device ? trustedBy(live?.trusted, device.id) : [];
  const waiting = device && trustedOn.length ? waitingForApproval(mine, device.id).filter((d) => !later.has(d.deviceId ?? '') && proven.has(d.deviceId ?? '')) : [];
  const others: MyDevice[] = mine.filter((d) => d.deviceId && d.deviceId !== device?.id).map((d) => ({
    id: d.deviceId ?? '', name: d.name ?? '', fingerprint: prints.get(d.deviceId ?? '') ?? '', lastUsedMs: num(d.lastUsedMs),
    trustedOn: trustedBy(live?.trusted, d.deviceId ?? ''), waiting: num(d.approvalAskedMs) > 0, removed: num(d.revokedMs) > 0,
  }));
  const asked = num(mine.find((d) => d.deviceId === device?.id)?.approvalAskedMs) > 0;
  // What the agents cost so far, asked for by the owner's window only (#1044).
  const usage = useUsageReport(api, live !== undefined, pairing.owner);
  useEffect(() => {
    if (!hub) return;
    // What was heard of machines and answered here belongs to one connection: another hub, or this one again, starts from nothing.
    forgetSkipCapable();
    forgetGiveSupport();
    const c = connectLive({ url: hub.url, token: hub.token, onView: (v) => setLive(shown(v)), onState: setLink, onPair: pairing.dispatch, onRole: setRole,
      onRejected: () => { forgetToken(); setLive(undefined); setHub(undefined); },
      onGrantRequired: grants.request });
    setApi(c);
    return () => { c.close(); setApi(undefined); setRole('unspecified'); };
  }, [hub, pairing.dispatch, grants.request]);
  // "Lock" in the title bar: this window's session ends at the hub and the sign-in screen comes back. Agents keep running.
  const lockNow = useCallback(() => {
    if (!hub) return;
    void endSession(hub.token);
    setLive(undefined);
    setHub(undefined);
  }, [hub]);
  // The idle lock does what Lock does: the session ends at the hub, so reloading the tab opens nothing; signing in again takes a code
  // (and the screen password from another machine) or a passkey.
  useEffect(() => {
    if (!hub || idle === 0) return;
    return watchIdle(idle, lockNow, window);
  }, [hub, idle, lockNow]);
  // ADR-0091: plan limits as the machines reported them (the demo data when there is no hub).
  const quotaList = useMemo(() => (hub ? (live ? quotaObs(live) : []) : demoQuota), [hub, live]);
  const quotaMachines = useMemo(() => new Map((live ? machines(live) : hub ? [] : demoMachines).map((m) => [m.id, m.name] as const)), [hub, live]);
  const agents = useMemo(() => (hub ? (live ? tiles(live) : []) : demoAgents), [hub, live]);
  // ADR-0095 step 2: "Give to an agent" on a Backlog task, from its board card or the command palette (a hub window only).
  // Step 3: "Start a new agent for this task" opens the ordinary start window with the same choices and the same signed command.
  const boardList = useMemo(() => (live ? boards(live) : []), [live]);
  const giveApi = useMemo<GiveApi | undefined>(() => {
    if (!hub || !api || !live || !changes) return undefined;
    const list = machines(live);
    return {
      agents, online: new Set(list.filter((m) => m.online).map((m) => m.id)), machines: list.map((m) => ({ id: m.id, name: m.name })), send: (agent, task) => giveTask(api.command, agent, task),
      start: {
        projects: projectChoices(live), machines: list, trust: device ? { id: device.id, key: device.publicKeyHex } : undefined,
        askApproval: device ? { can: (node) => canBeApprovedOn(live.trusted, node, mine, device.id), ask: askApproval } : undefined,
        loadKinds, inspect: inspectDirectory, list: listDirectory, launch: launchAgent,
      },
    };
  }, [hub, api, live, changes, agents, device, mine, askApproval, loadKinds, inspectDirectory, listDirectory, launchAgent]);
  const giving = useGiveAsk();
  // What was heard of machines that do not take tasks belongs to one connection, and to a machine until it goes offline (an update restarts it).
  useEffect(() => { if (link === 'live') forgetGiveSupport(); }, [link]);
  useEffect(() => { if (giveApi?.online) forgetOffline(giveApi.online); }, [giveApi?.online]);
  const giveSupport = useGiveSupport(); // machines heard not to take tasks drop out of the palette too
  const current = hash.startsWith('#/agent/') ? decodeHash(hash.slice('#/agent/'.length)) : undefined;
  const approvalsNow = live ? approvalList(live) : hub ? [] : demoApprovals;
  // On a phone a notice's link to an approval opens the agent that asks (ADR-0046, ADR-0067); answered already, the list.
  const redirect = phone ? approvalRedirect(phoneScreen(hash), approvalsNow, !hub || live !== undefined) : undefined;
  useEffect(() => { if (redirect) window.location.replace(redirect); }, [redirect]);

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

  const loadDiff = useCallback(async (a: AgentTileModel, path = ''): Promise<DiffSource> => {
    if (!api || !a.workspaceRef) return '';
    const ok = await api.command(a.workspaceRef.node, a.workspaceRef.directory ? { workspaceQuery: { workspaceId: a.workspaceRef.id, operation: 'diff', path } } : { gitDiff: { worktreeId: a.workspaceRef.id, path } });
    const diff = ok.gitDiff?.diff ?? '';
    const secret = (ok.gitDiff?.secretFiles ?? []).map((c) => ({ path: c.path ?? '', added: Number(c.added ?? 0), removed: Number(c.removed ?? 0) }));
    return secret.length > 0 ? { diff, secretFiles: secret } : diff;
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
          return (ok.filesList?.entries ?? []).map((e) => ({ name: e.name ?? '', isDir: !!e.isDir, size: Number(e.sizeBytes ?? 0), ...(e.secret ? { secret: true } : {}) }));
        },
        read: async (path) => (await api.command(w.node, w.directory ? { workspaceQuery: { workspaceId: w.id, operation: 'read', path } } : { filesRead: { worktreeId: w.id, path } })).filesRead?.data ?? new Uint8Array(),
      };
      fileAccess.current.set(key, f);
    }
    return f;
  }, [api]);

  // ADR-0090: an agent whose node answered a prompt in the rich form (`AgentPromptResult`, capability `chat.rich`) is remembered, so the
  // window offers Stop turn for it. A node without the capability answers with nothing and adds none.
  const [rich, setRich] = useState<ReadonlySet<string>>(() => new Set());
  const sendPrompt = useCallback(async (a: AgentTileModel, text: string): Promise<PromptResult> => {
    if (!api || !a.workspaceRef) return { ok: false };
    try {
      const ok = await api.command(a.workspaceRef.node, { agentPrompt: { agentId: a.id.slice(a.id.indexOf('/') + 1), text } });
      const r = ok.agentPrompt;
      if (r) setRich((s) => (s.has(a.id) ? s : new Set(s).add(a.id)));
      return { ok: true, queued: !!r?.queued };
    } catch (e) { return { ok: false, code: (e as { code?: string }).code }; }
  }, [api]);

  // ADR-0090: the agent's own settings (model, mode, effort, ...) and ending its turn. Both are the user's own commands, addressed by the
  // hub-wide key the choice was made for; a node without the capability answers `protocol.capability_missing`, which reads as "no".
  const setConfig = useCallback(async (agentKey: string, configId: string, value: string): Promise<ActionResult> => {
    const to = agentTarget(agentKey);
    if (!api || !to) return false;
    try { await api.command(to.node, { agentSetConfig: { agentId: to.agentId, configId, value } }); return true; } catch (e) { return failure(to.node, e); }
  }, [api]);
  const cancelTurn = useCallback(async (agentKey: string, dropQueue: boolean): Promise<ActionResult> => {
    const to = agentTarget(agentKey);
    if (!api || !to) return false;
    try { await api.command(to.node, { agentCancelTurn: { agentId: to.agentId, dropQueue } }); return true; } catch (e) { return (e as { code?: string }).code ?? false; }
  }, [api]);
  // The approvals that wait now (hub-wide keys), so a control that sent a switch knows when its card is gone.
  const waitingApprovals = useMemo<ReadonlySet<string>>(() => new Set(live ? [...live.approvals.keys()] : []), [live]);
  const chatActions = useMemo<ChatActions>(() => (hub && api && changes ? { setConfig, cancelTurn, rich, approvals: waitingApprovals } : {}), [hub, api, changes, setConfig, cancelTurn, rich, waitingApprovals]);

  // ADR-0058: the user's own switch of a chat agent's mode or model, among what the agent offered.
  // ADR-0058: the user's own switch of a chat agent's mode or model, among what the agent offered. Addressed by the hub-wide key the
  // choice was made for (never by whichever agent is on screen when the request goes out).
  const chooseOption = useCallback(async (agentKey: string, kind: 'mode' | 'model', id: string): Promise<ActionResult> => {
    const to = agentTarget(agentKey);
    if (!api || !to) return false;
    try {
      await api.command(to.node, kind === 'mode' ? { agentSetMode: { agentId: to.agentId, mode: id } } : { agentSetModel: { agentId: to.agentId, model: id } });
      return true;
    } catch (e) { return failure(to.node, e); }
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
  // The agent a grant question names: its tile's name, or its id before the list has it.
  const grantAgentId = grantAsk?.ask.scope.startsWith('agent:') ? `${grantAsk.node}/${grantAsk.ask.scope.slice('agent:'.length)}` : '';
  const grantAgentName = agents.find((a) => a.id === grantAgentId)?.name ?? grantAgentId.slice(grantAgentId.indexOf('/') + 1);

  const open = useCallback((id: string) => go(`#/agent/${encodeURIComponent(id)}`), [go]);
  const next = useCallback(() => {
    const n = nextNeedingMe(agents, current);
    if (n) open(n.id);
  }, [agents, current, open]);

  // What needs the person (the one list the console, the panel and the phone bar show), for the attention layer's card, banner and notice.
  const attention = useMemo(() => attentionList(approvalsNow, agents, pairing.ui?.cards ?? []), [approvalsNow, agents, pairing.ui?.cards]);
  // The full-size view and the phone's agent screen show one agent: it is the one on screen (the window's panes say theirs in Console).
  useOnScreen(current, current === undefined ? [] : [current]);
  // T44 (#1048): one set of answer holds for the whole window, so a request answered on one surface (a pane's card, the side panel, Home,
  // the attention dialog, a switch's card) is held on every other; an answer that went is forgotten when its request leaves this list.
  const heldIds = useMemo(() => approvalsNow.map((x) => x.id), [approvalsNow]);
  const reviewAttention = useCallback((it: (typeof attention)[number]) => {
    const agent = it.kind === 'pair' ? undefined : it.agent;
    if (agent) { open(agent.id); return; }
    // My machines shows every request to join in full: reviewing one there brings its card into view instead of leaving the page.
    const card = it.kind === 'pair' && isMachinesHash(hash) ? document.getElementById(`pair-request-${it.card.id}`)?.closest<HTMLElement>('section') : undefined;
    if (card) { card.scrollIntoView?.({ block: 'center' }); card.focus({ preventScroll: true }); return; }
    go('#/');
    if (!phone) requestPanel('fleet');
  }, [go, open, phone, hash]);
  // The theme editor is a panel of the window: from any other page, back to the window first.
  // A phone has no panels: there the editor is a screen of its own (ADR-0067 rung 2).
  const editTheme = useCallback(() => { if (phone) { go('#/theme'); return; } if (!['', '#/', '#/demo'].includes(hash)) go('#/'); requestPanel('theme'); }, [go, hash, phone]);
  // Home is what Console / PhoneHome draw (the chain below); any other page gets the first request as a floating card.
  const settingsDemo = hash === '#/gallery/settings' || hash.startsWith('#/gallery/settings/');
  const showsHome = current === undefined && hash !== '#/gallery' && !settingsDemo && !isSettingsHash(hash) && !isMachinesHash(hash) && !(hash === '#/audit' && hub) && !(hash === '#/theme' && phone);
  // "Allow a new machine to join": back to Home, where the window for new machines is; its own button asks, this only brings it into view.
  const showPairing = useCallback(() => {
    go('#/');
    if (!phone) requestPanel('fleet');
    window.setTimeout(() => document.querySelector<HTMLElement>('.pair-window button:not(:disabled)')?.focus(), 50);
  }, [go, phone]);
  // Back to the window (the panes live there); from the window itself (the demo included) nothing moves.
  const toWindow = useCallback(() => { if (!['', '#/', '#/demo'].includes(hash)) go('#/'); }, [hash, go]);
  // The palette's searchable objects beside the commands: machines and projects by name (paletteExtras), and its three actions, which are
  // commands here (machines.allow, window.lock, go.settings) offered exactly where paletteExtras offers them.
  const extras = paletteExtras({ machines: live ? machines(live) : hub ? [] : demoMachines, projects: projectsOf(agents), hub: !!hub, phone, passwordSet: info.passwordSet, canPair: !!pairing.ui?.onOpen, go,
    showProject: (id) => { go('#/'); requestProject(id); }, showPairing, lock: hub ? lockNow : undefined });
  const extra = (id: string): Handler | false => { const it = extras.find((x) => x.id === id); return it ? { run: it.run } : false; };
  // The window's commands this part runs (docs/ux/keybindings.md); the window, the side panel and plugins register theirs.
  useCommands({
    'palette.open': { run: () => setPalette((p) => !p) },
    'attention.next': { run: next },
    'go.home': { run: () => { go('#/'); if (!phone) requestPanel('fleet'); } },
    'agent.start': hub && changes ? { run: () => { toWindow(); openStart(); } } : false,
    // The tools open as panes of the window, beside the agent in focus (not on a phone, which has no panes).
    ...Object.fromEntries(TOOL_PANELS.map((panel) => [`tool.${panel}`, phone ? false : { run: () => { toWindow(); requestTool({ panel }); } }])),
    'terminal.new': !phone && hub && changes ? { run: () => { toWindow(); openStart(undefined, true); } } : false,
    'go.machines': { run: () => go('#/machines') },
    'machines.allow': extra('pair'),
    'window.lock': extra('lock'),
    'go.settings': extra('settings'),
    'go.keyboard': { run: () => go('#/device/keyboard') },
    'go.audit': hub ? { run: () => go('#/audit') } : false,
    // The language, here too: on a phone the title bar has no room for its picker.
    ...Object.fromEntries((['system', 'en', 'tr'] as const).map((l) => [`language.${l}`, { run: () => { chooseLanguage(l); } }])),
    'theme.editor': { run: editTheme },
    // Plugins loaded from a local file, and safe mode (ADR-0114): the safe-mode key itself is heard before anything else (plugins/safe.ts),
    // so its command is here for the palette and its keys are fixed.
    'plugins.open': { run: () => setPluginsOpen(true) },
    'window.safe-mode': SAFE ? false : { run: () => location.assign(safeAddress(location.href)) },
  });
  const keymap = useKeymap();
  const commandsVersion = useCommandsVersion();
  const items: PaletteItem[] = useMemo(() => {
    // Every command on screen that can act now, with its key; in the order the palette listed them before rung 4, then the rest in the
    // registry's order.
    const placed = new Set<string>();
    const cmd = (id: string): PaletteItem[] => {
      placed.add(id);
      const def = commandById(id);
      const h = handlerOf(id);
      if (!def || def.notInPalette || !h || h.available === false) return [];
      return [{ id, label: h.label ?? commandLabel(def), run: () => { runCommand(id); }, group: def.group, icon: def.icon, keys: shownKey(keymap, id), ariaKeys: ariaKey(keymap, id) }];
    };
    const objects = extras.filter((x) => !['pair', 'lock', 'settings'].includes(x.id));
    return [
      ...cmd('attention.next'), ...cmd('go.home'), ...cmd('agent.start'), ...TOOL_PANELS.flatMap((p) => cmd(`tool.${p}`)), ...cmd('terminal.new'), ...cmd('go.machines'),
      ...objects, ...cmd('machines.allow'), ...cmd('window.lock'), ...cmd('go.settings'), ...cmd('go.audit'),
      ...(['system', 'en', 'tr'] as const).flatMap((l) => cmd(`language.${l}`)), ...cmd('theme.editor'), ...cmd('plugins.open'), ...cmd('window.safe-mode'),
      ...themeStore.themes().map((x) => ({ id: `theme-${x.id}`, label: t('palette-theme', { name: themeName(x) }), run: () => themeStore.choose(x.id), group: 'appearance' as const, icon: 'sun' as const })),
      ...agents.map((a) => ({ id: `agent-${a.id}`, label: t('palette-open-agent', { name: a.name }), run: () => open(a.id), group: 'agents' as const, agent: { accent: a.accent, kind: a.kind || a.name } })),
      // Close agent: opens that agent and asks for the confirm there; nothing is closed from the palette itself.
      ...(hub && api && changes ? agents.filter((a) => !a.closed && a.machineId).map((a) => ({ id: `close-${a.id}`, label: t('palette-close-agent', { name: a.name }), run: () => { open(a.id); closeAsk.ask(a.id, 'pane'); }, group: 'agents' as const, agent: { accent: a.accent, kind: a.kind || a.name } })) : []),
      // A Backlog task is a searchable object: choosing it opens "Give to an agent" for it (the dialog asks before anything is sent).
      ...(giveApi ? givableTasks(boardList, giveApi.agents, giveApi.online, undefined, (id) => !!giveApi.start && canStartFor(giveApi.start.projects, giveApi.start.machines, id)).map((task) => ({ id: `give-${task.key}`, label: t('palette-give-task', { title: plain(task.title, 80) || t('board-task-untitled') }), run: () => giveAsk.ask(task), group: 'tasks' as const, icon: 'board' as const })) : []),
      ...COMMANDS.filter((c) => !placed.has(c.id)).flatMap((c) => cmd(c.id)),
    ];
  }, [api, agents, hub, changes, open, language, palette, live, info.passwordSet, pairing.ui, giveApi, boardList, giveSupport, keymap, commandsVersion, phone]); // eslint-disable-line react-hooks/exhaustive-deps

  // The one key listener of the window (docs/ux/keybindings.md): the keys in force, looked up on every key press.
  useEffect(() => listenForKeys({ map: () => keyStore.state().map, mac: IS_MAC }), []);

  // ADR-0061: a machine that reported whom it trusts, and not this browser; one that has not reported is not called untrusted.
  const untrustedOn = hub && device && live?.trusted ? (node: string) => { const ids = live.trusted?.get(node); return !!ids && !ids.includes(device.id); } : undefined;
  const startWindow = !hub && import.meta.env.DEV && starting ? <StartAgent initialNode={starting.node} initialKind={starting.kind} projects={demoProjects} machines={demoMachines}
    loadKinds={async () => [{ kind: 'claude-code', name: 'Claude Code', installed: true, level: 3 }, { kind: 'codex', name: 'Codex', installed: true, level: 2 }, { kind: 'shell', name: 'Shell', installed: true, level: 1 }, { kind: 'gemini', name: 'Gemini CLI', installed: false, level: 2 }]}
    list={async (_n, path) => demoList(path)} inspect={async (_n, path) => ({ path, projectId: 'demo', gitRoot: path, branch: 'main', portable: false })} onStart={async () => { setStarting(false); return undefined; }} onClose={() => setStarting(false)} /> : starting && live ? <StartAgent initialNode={starting ? starting.node : undefined} initialKind={starting ? starting.kind : undefined} trust={device ? { id: device.id, key: device.publicKeyHex } : undefined}
    askApproval={device ? { can: (node) => canBeApprovedOn(live.trusted, node, mine, device.id), ask: askApproval } : undefined} projects={projectChoices(live)} machines={machines(live)} inspect={inspectDirectory} list={listDirectory} loadKinds={loadKinds} onStart={startAgent} onClose={() => setStarting(false)} /> : undefined;

  // Served by the hub (a production build) and not signed in: ask for the code. Development keeps the demo data.
  if (!hub && !import.meta.env.DEV && hash !== '#/gallery' && hash !== '#/demo' && !settingsDemo) {
    return <SignInPage onSubmit={trySignIn} onPasskey={passkeysSupported() ? trySignInPasskey : undefined} notice={signInNotice ? t(signInNotice) : undefined} passwordRequired={info.passwordRequired} key={linkCode ?? ''} initialCode={linkCode} />;
  }

  return (
    <AnswerHoldsProvider listed={heldIds}>
    <QuotaProvider obs={quotaList} machines={quotaMachines}>
    <CloseAgentContext.Provider value={hub && api && changes ? closeAgent : undefined}>
    <ChatActionsContext.Provider value={chatActions}>
    <GiveContext.Provider value={giveApi}>
      {/* The Start window is drawn by Home only: My machines goes to the window first, as the palette does, so it opens at once. */}
      {hash === '#/gallery' ? <Gallery /> : settingsDemo ? <SettingsDemo section={hash.split('/')[3] || undefined} onBack={() => go('#/gallery')} /> : isMachinesHash(hash) ? <MachinesPage focus={machineOfHash(hash)} machines={live ? machines(live) : hub ? [] : demoMachines} agents={agents} onBack={() => go('#/')} onForget={hub && ownsMachines(role) ? forgetMachines : undefined}
        pairing={pairing.ui} onStart={hub && api && live && changes ? (node) => { toWindow(); openStart(node); } : undefined} onNewTerminal={hub && api && live && changes ? (node) => { toWindow(); openStart(node, true); } : undefined} /> : hash === '#/audit' && hub ? <AuditPage load={(who) => loadAudit(hub.token, who)} onBack={() => go('#/')} /> : isSettingsHash(hash) ? <DevicePage section={settingsSectionOf(hash)} device={device} onBack={() => go('#/')} onTheme={editTheme} onAddPasskey={hub && passkeysSupported() ? (password) => addPasskey(hub.token, undefined, password) : undefined} passkeyNeedsPassword={info.passwordRequired}
          fingerprint={device ? prints.get(device.id) : undefined} machines={live ? machines(live) : undefined} trustedOn={trustedOn} others={hub ? others : undefined}
          loadKinds={hub && api && live ? loadKinds : undefined} asked={asked} onAsk={hub && api ? askApproval : undefined} onRemove={hub && api && changes ? removeDevice : undefined}
          security={hub ? <ScreenPasswordSettings passwordSet={info.passwordSet} idle={idle} onIdle={(m) => { setIdleMinutes(m); setIdle(m); }}
            onSet={async (password, current) => { const r = await setPassword(hub.token, password, current); void refreshInfo(); return r; }} /> : undefined} /> : hash === '#/theme' && phone ? <PhoneTheme onBack={() => go('#/')} />
        : current !== undefined && phone
        ? <PhoneAgent agent={selectedAgent} agents={agents} approvals={approvalsNow} loadDiff={hub && api ? loadDiff : !hub ? demoLoadDiff : undefined} filesOf={hub && api ? filesOf : !hub ? demoFilesOf : undefined} machines={live ? machines(live) : hub ? [] : demoMachines} onResolve={answerRequest} untrusted={untrustedOn} notice={notice ? t(notice) : undefined}
          source={terminal} chatSource={selectedAgent?.terminal === false ? chatOf(selectedAgent) : undefined} sendPrompt={hub && api && changes ? sendPrompt : undefined} chooseOption={hub && api && changes ? chooseOption : undefined} link={hub ? link : 'live'} onOpen={open} onBack={() => go('#/')} />
        : current !== undefined
        ? <Focus agent={selectedAgent} approvals={live ? approvalList(live) : hub ? [] : demoApprovals} onResolve={answerRequest} stale={hub ? link !== 'live' : false} chatSource={selectedAgent?.terminal === false ? chatOf(selectedAgent) : undefined} sendPrompt={hub && api && changes ? sendPrompt : undefined} chooseOption={hub && api && changes ? chooseOption : undefined} source={terminal} onBack={() => go('#/')} />
        : phone
        ? <PhoneHome agents={agents} machines={live ? machines(live) : hub ? [] : demoMachines} approvals={approvalsNow} onResolve={answerRequest} untrusted={untrustedOn} notice={notice ? t(notice) : undefined}
          link={hub ? link : 'live'} onRetry={api ? api.retry : undefined} onOpen={open} onCommand={() => setPalette(true)} onLock={hub ? lockNow : undefined} start={startWindow} pairing={pairing.ui} guest={pairing.owner === 'no'} watching={!changes}
          onStart={hub && api && live && changes ? () => openStart() : (!hub && import.meta.env.DEV ? () => openStart() : undefined)} />
        : <Console agents={agents} machines={live ? machines(live) : []} usage={usage} projects={live && usage ? costByProject(live, usage.perAgent) : []}
          approvals={live ? approvalList(live) : hub ? [] : demoApprovals} messages={live ? recentMessages(live) : []} boards={boardList} terminalFor={terminalFor} chatOf={hub && api ? chatOf : undefined} sendPrompt={hub && api && changes ? sendPrompt : undefined} chooseOption={hub && api && changes ? chooseOption : undefined} loadDiff={hub && api ? loadDiff : !hub ? demoLoadDiff : undefined} loadHistory={hub && api ? loadHistory : undefined} filesOf={hub && api ? filesOf : !hub ? demoFilesOf : undefined} onOpen={open}
          onResolve={answerRequest} helperGrants={hub && api ? helperGrants : undefined} revokeGrant={hub && api && changes ? revokeGrant : undefined} notice={notice ? t(notice) : undefined} link={hub ? link : 'live'} onRetry={api ? api.retry : undefined}
          onStart={hub && api && live && changes ? (node) => openStart(node) : (!hub && import.meta.env.DEV ? (node) => openStart(node) : undefined)}
          onNewTerminal={hub && api && live && changes ? (node) => openStart(node, true) : (!hub && import.meta.env.DEV ? (node) => openStart(node, true) : undefined)} onCommand={() => setPalette(true)} onForget={hub && api && changes ? forgetAgent : undefined} onResume={hub && api && changes ? resumeAgent : (!hub && import.meta.env.DEV ? async () => undefined : undefined)} onLock={hub ? lockNow : undefined}
          start={startWindow} pairing={pairing.ui} guest={pairing.owner === 'no'} watching={!changes} />}
      {grantAsk && <GrantDialog machine={live?.nodes.get(grantAsk.node)?.hostname || grantAsk.node} agent={grantAgentName} kind={grantAsk.ask.kind} busy={grantAsk.busy} failed={grantAsk.failed} onAllow={() => void grants.allow()} onClose={grants.dismiss} />}
      {/* Away from Home (an agent, a page) a request to join still reaches the owner: the first card floats over the window. My machines
          shows every request in full itself, so nothing floats there. */}
      {pairing.ui && pairing.ui.cards[0] && !showsHome && !isMachinesHash(hash) && <PairRequest key={pairing.ui.cards[0].id} request={pairing.ui.cards[0]} offline={pairing.ui.offline} onAnswer={(a, r) => pairing.ui!.onAnswer(pairing.ui!.cards[0]!, a, r)} />}
      {/* The demo Settings page shows Settings only: no demo requests over it. */}
      <AgentAlerts agents={agents} attention={attention} ready={!hub || live !== undefined} />
      <AttentionLayer items={settingsDemo && !hub ? [] : attention} stale={hub ? link !== 'live' : false} phone={phone} onResolve={answerRequest} onReview={reviewAttention} />
      {palette && <CommandPalette items={items} onClose={() => setPalette(false)} />}
      {pluginsOpen && <PluginsDialog onClose={() => setPluginsOpen(false)} />}
      {SAFE && <SafeModeNote />}
      {giving && giveApi && <GiveDialog key={giving.key} task={giving} give={giveApi} onClose={giveAsk.cancel} />}
      {hub && live && changes && waiting[0] && <DeviceRequest key={waiting[0].deviceId ?? ''}
        request={{ id: waiting[0].deviceId ?? '', name: deviceLabel(waiting[0].name, waiting[0].deviceId ?? ''), fingerprint: prints.get(waiting[0].deviceId ?? '') ?? '' }}
        machines={machines(live).filter((m) => trustedOn.includes(m.id)).map((m) => ({ id: m.id, name: m.name }))}
        onApprove={(nodes) => approveDevice(waiting[0]!, nodes)}
        onLater={() => setLater((l) => new Set(l).add(waiting[0]?.deviceId ?? ''))} />}
    </GiveContext.Provider>
    </ChatActionsContext.Provider>
    </CloseAgentContext.Provider>
    </QuotaProvider>
    </AnswerHoldsProvider>
  );
}

// The demo window (no hub): the login-fix workspace of the demo agents, so the review screen, the file browser and the tool panes can be
// seen and tested. Another demo workspace is a clean, empty folder.
const demoLoadDiff = async (a: AgentTileModel, path?: string) => (a.workspaceRef?.id === 'login-fix' ? demoDiff(path) : '');
const emptyFiles: WorkspaceFiles = { list: async () => [], read: async () => { throw new Error('not found'); } };
const demoFilesOf = (a: AgentTileModel) => (a.workspaceRef ? (a.workspaceRef.id === 'login-fix' ? demoFiles : emptyFiles) : undefined);
