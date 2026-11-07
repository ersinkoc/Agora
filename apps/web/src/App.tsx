import { useCallback, useEffect, useMemo, useState } from 'react';
import { CommandPalette, type PaletteItem } from './components/CommandPalette';
import { demoAgents } from './fixtures';
import { t } from './i18n';
import { nextNeedingMe } from './model';
import { connectLive, type Live, type LinkState } from './net/live';
import type { TerminalSource } from './terminal/source';
import { machinesOnline, tiles, type View } from './net/view';
import { hubConnection } from './net/session';
import { Focus, Gallery, Home } from './screens';

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
  const hub = useMemo(hubConnection, []);
  const [live, setLive] = useState<View | undefined>();
  const [link, setLink] = useState<LinkState>('connecting');
  const [api, setApi] = useState<Live | undefined>();
  useEffect(() => {
    if (!hub) return;
    const c = connectLive({ url: hub.url, token: hub.token, onView: (v) => setLive({ ...v }), onState: setLink });
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
    ...agents.map((a) => ({ id: `agent-${a.id}`, label: t('palette-open-agent', { name: a.name }), run: () => open(a.id) })),
  ], [agents, go, next, open]);

  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName);
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); setPalette((p) => !p); }
      else if (!typing && !e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === 'n') next();
    };
    window.addEventListener('keydown', on);
    return () => window.removeEventListener('keydown', on);
  }, [next]);

  return (
    <>
      {hash === '#/gallery' ? <Gallery /> : current !== undefined
        ? <Focus agent={agents.find((a) => a.id === current)} source={terminal} onBack={() => go('#/')} />
        : <Home onOpen={open} agents={agents} machines={live ? machinesOnline(live) : undefined} link={hub ? link : 'live'} />}
      {palette && <CommandPalette items={items} onClose={() => setPalette(false)} />}
    </>
  );
}
