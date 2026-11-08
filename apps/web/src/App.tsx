import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CommandPalette, type PaletteItem } from './components/CommandPalette';
import { demoAgents } from './fixtures';
import { t } from './i18n';
import { nextNeedingMe } from './model';
import { connectLive, type Live, type LinkState } from './net/live';
import type { TerminalSource } from './terminal/source';
import { StartAgent, type KindChoice } from './components/StartAgent';
import { approvalList, projectChoices, recentMessages, machines, machinesOnline, tiles, type ApprovalModel, type View } from './net/view';
import { freshApprovals, titleWithCount } from './net/alert';
import { indexedDbStore, signApproval, thisDevice, type Device } from './net/approval';
import { addPasskey, passkeysSupported, signInWithPasskey } from './net/passkey';
import { summarize, type UsageSummary } from './net/usage';
import { forgetToken, hubConnection, readCode, signIn } from './net/session';
import { DevicePage, Focus, Gallery, Home, MachinesPage, SignInPage } from './screens';

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
  const [hash, go] = useHash();
  const [palette, setPalette] = useState(false);
  // With a hub token the fleet comes from the hub; without one (development) the demo data stands in.
  const [hub, setHub] = useState(hubConnection);
  const [signInNotice, setSignInNotice] = useState<string | undefined>();
  const trySignIn = useCallback(async (code: string) => {
    const r = await signIn(code);
    if (r.ok) { setSignInNotice(undefined); setHub(hubConnection()); return; }
    setSignInNotice(t(r.reason === 'refused' ? 'signin-refused' : r.reason === 'too-many' ? 'signin-too-many' : 'signin-failed'));
  }, []);
  const trySignInPasskey = useCallback(async () => {
    const r = await signInWithPasskey();
    if (r.ok) { setSignInNotice(undefined); setHub(hubConnection()); return; }
    if (r.reason !== 'cancelled') setSignInNotice(t(r.reason === 'too-many' ? 'signin-too-many' : 'signin-passkey-failed'));
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
  const [starting, setStarting] = useState(false);
  const loadKinds = useCallback(async (node: string): Promise<KindChoice[]> => {
    const ok = await api!.command(node, { agentKinds: {} });
    return (ok.agentKinds?.kinds ?? []).map((k) => ({ kind: k.kind ?? '', name: k.displayName || (k.kind ?? ''), installed: !!k.installed }));
  }, [api]);
  const startAgent = useCallback(async (node: string, projectId: string, kind: string): Promise<string | undefined> => {
    try {
      const ok = await api!.command(node, { agentStart: { projectId, kind } });
      const id = ok.agentStart?.agentId;
      setStarting(false);
      if (id) window.location.hash = `#/agent/${encodeURIComponent(`${node}/${id}`)}`;
      return undefined;
    } catch (e) {
      const code = (e as { code?: string }).code;
      return t(code === 'agent.kind_unknown' ? 'start-kind-unknown' : code === 'node.resource_limit' ? 'start-limit' : 'start-failed');
    }
  }, [api]);
  useEffect(() => {
    const store = indexedDbStore();
    if (store) void thisDevice(store).then(setDevice).catch(() => setDevice(undefined));
  }, []);
  const resolve = useCallback(async (a: ApprovalModel, approved: boolean) => {
    if (!api || !device) { setNotice(t('approval-failed')); return; }
    setNotice(undefined);
    try {
      const signature = await signApproval(device, a.node, a.localId, approved);
      await api.command(a.node, { approvalResolve: { approvalId: a.localId, approved, signerDeviceId: device.id, signature } });
    } catch (e) {
      const code = (e as { code?: string }).code;
      setNotice(code === 'approval.signature_invalid' ? t('approval-not-trusted') : t('approval-failed'));
    }
  }, [api, device]);
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
      onRejected: () => { forgetToken(); setLive(undefined); setHub(undefined); } });
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

  const open = useCallback((id: string) => go(`#/agent/${encodeURIComponent(id)}`), [go]);
  const next = useCallback(() => {
    const n = nextNeedingMe(agents, current);
    if (n) open(n.id);
  }, [agents, current, open]);

  const items: PaletteItem[] = useMemo(() => [
    { id: 'next', label: t('palette-next-attention'), run: next },
    { id: 'home', label: t('palette-go-home'), run: () => go('#/') },
    ...(hub ? [{ id: 'start', label: t('action-start-agent'), run: () => { go('#/'); setStarting(true); } }] : []),
    { id: 'machines', label: t('palette-go-machines'), run: () => go('#/machines') },
    ...(hub ? [{ id: 'device', label: t('device-link'), run: () => go('#/device') }] : []),
    ...agents.map((a) => ({ id: `agent-${a.id}`, label: t('palette-open-agent', { name: a.name }), run: () => open(a.id) })),
  ], [agents, go, hub, next, open]);

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
    return <SignInPage onSubmit={trySignIn} onPasskey={passkeysSupported() ? trySignInPasskey : undefined} notice={signInNotice} />;
  }

  return (
    <>
      {hash === '#/gallery' ? <Gallery /> : hash === '#/machines' ? <MachinesPage machines={live ? machines(live) : []} usage={usage} onBack={() => go('#/')} /> : hash === '#/device' ? <DevicePage device={device} onBack={() => go('#/')} onAddPasskey={hub && passkeysSupported() ? async () => (await addPasskey(hub.token)).ok : undefined} /> : current !== undefined
        ? <Focus agent={agents.find((a) => a.id === current)} costMicroUsd={current ? usage?.perAgent.get(current) : undefined} source={terminal} onBack={() => go('#/')} />
        : <Home onOpen={open} agents={agents} machines={live ? machinesOnline(live) : undefined} link={hub ? link : 'live'} approvals={live ? approvalList(live) : []} messages={live ? recentMessages(live) : []} onResolve={resolve} notice={notice}
          onStart={hub && api && live ? () => setStarting(true) : undefined}
          start={starting && live ? <StartAgent projects={projectChoices(live)} loadKinds={loadKinds} onStart={startAgent} onClose={() => setStarting(false)} /> : undefined} />}
      {palette && <CommandPalette items={items} onClose={() => setPalette(false)} />}
    </>
  );
}
