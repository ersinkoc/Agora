import { useCallback, useEffect, useMemo, useState } from 'react';
import { CommandPalette, type PaletteItem } from './components/CommandPalette';
import { demoAgents } from './fixtures';
import { t } from './i18n';
import { nextNeedingMe } from './model';
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
  const agents = demoAgents;
  const current = hash.startsWith('#/agent/') ? decodeURIComponent(hash.slice('#/agent/'.length)) : undefined;

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
        ? <Focus agent={agents.find((a) => a.id === current)} onBack={() => go('#/')} />
        : <Home onOpen={open} />}
      {palette && <CommandPalette items={items} onClose={() => setPalette(false)} />}
    </>
  );
}
