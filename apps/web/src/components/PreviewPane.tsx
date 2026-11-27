import { useEffect, useId, useMemo, useState } from 'react';
import { t } from '../i18n';
import type { PreviewActions, PreviewState } from '../layout/panels';
import type { AgentTileModel } from '../model';
import { entryUrl, originUrl, previewErrorKey, type Forward } from '../net/preview';
import type { MachineModel } from '../net/view';
import { visible } from '../net/view';
import { Icon } from './Icon';
import { Button, SegmentedControl } from './kit';

type Size = 'full' | 'tablet' | 'phone';
const WIDTH: Record<Size, string | undefined> = { full: undefined, tablet: '820px', phone: '390px' };
const HOURS = [1, 8, 24] as const;
const keyOf = (f: Pick<Forward, 'node' | 'id'>) => `${f.node}/${f.id}`;
const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

// ADR-0088 step 4: the Preview pane. The forwards this browser opened are shown in a frame on their own origin (a port of the hub's),
// entered with a ticket the hub gives only to their opener; a port is opened on a machine with this device's signature; the machines'
// "my app is on port N" suggestions are cards with an Open button (never an open by themselves). The frame is sandboxed without
// `allow-popups-to-escape-sandbox`, and its address is the path this pane set (a page on another origin cannot be read).
export function PreviewPane({ state, actions, machines, fleet }: {
  state?: PreviewState; actions?: PreviewActions; machines: readonly MachineModel[]; fleet: readonly AgentTileModel[];
}) {
  const id = useId();
  const mine = useMemo(() => (state?.forwards ?? []).filter((f) => f.creator === state?.device), [state]);
  const others = useMemo(() => (state?.forwards ?? []).filter((f) => f.creator !== state?.device), [state]);
  const [chosen, setChosen] = useState<string | undefined>();
  const current = mine.find((f) => keyOf(f) === chosen) ?? mine[0];
  const [src, setSrc] = useState<{ key: string; url: string; port: number; n: number } | undefined>();
  const [path, setPath] = useState('/');
  const [size, setSize] = useState<Size>('full');
  const [error, setError] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);
  const online = machines.filter((m) => m.online);
  const [node, setNode] = useState(online[0]?.id ?? '');
  const [port, setPort] = useState('');
  const [hours, setHours] = useState<number>(8);
  const [adding, setAdding] = useState(false);

  // Entering the forward in view (again when it changes): a fresh ticket, single use.
  useEffect(() => {
    if (!current || !actions || !state) return;
    if (src?.key === keyOf(current)) return;
    let live = true;
    void actions.enter(current).then((r) => {
      if (!live) return;
      if (r.ok) { setSrc({ key: keyOf(current), url: entryUrl(state.hostname, r.port, r.ticket), port: r.port, n: 0 }); setPath('/'); setError(undefined); }
      else setError(previewErrorKey(r.code, r.reason));
    });
    return () => { live = false; };
  }, [current, actions, state, src?.key]);

  if (!state || !actions) return <div className="preview-pane preview-pane--empty"><p className="muted m-0">{t('preview-unavailable')}</p></div>;

  const machineName = (n: string) => visible(machines.find((m) => m.id === n)?.name ?? n);
  const agentName = (key: string) => visible(fleet.find((a) => a.id === key)?.name ?? '');
  const open = async (n: string, p: number) => {
    setBusy(true);
    setError(undefined);
    const r = await actions.open(n, p, hours * 3600);
    setBusy(false);
    if (!r.ok) { setError(previewErrorKey(r.code, r.reason)); return; }
    setChosen(keyOf(r.forward));
    setPort('');
    setAdding(false);
  };
  const portNumber = Number(port);
  const portOk = Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535;
  const go = () => { if (src && state) setSrc({ ...src, url: originUrl(state.hostname, src.port, path || '/'), n: src.n + 1 }); };
  // The tab opens in the click itself (a browser blocks one opened after a wait), cut off from this window, and goes to the entry once
  // the ticket comes.
  const newTab = async () => {
    if (!current) return;
    const tab = window.open('', '_blank');
    if (tab) tab.opener = null;
    const r = await actions.enter(current);
    if (r.ok && tab) tab.location.href = entryUrl(state.hostname, r.port, r.ticket);
    else {
      tab?.close();
      if (!r.ok) setError(previewErrorKey(r.code, r.reason));
    }
  };
  const close = async (f: Forward) => {
    const code = await actions.close(f);
    if (code) setError(previewErrorKey(code));
    if (src?.key === keyOf(f)) setSrc(undefined);
  };

  const form = (
    <form className="preview-open" aria-label={t('preview-open-title')} onSubmit={(e) => { e.preventDefault(); if (portOk && node && !busy) void open(node, portNumber); }}>
      <h3 className="preview-open__title">{t('preview-open-title')}</h3>
      <label htmlFor={`${id}-node`}>{t('preview-machine')}</label>
      <select id={`${id}-node`} value={node} onChange={(e) => setNode(e.target.value)}>
        {online.map((m) => <option key={m.id} value={m.id}>{visible(m.name)}</option>)}
      </select>
      <label htmlFor={`${id}-port`}>{t('preview-port')}</label>
      <input id={`${id}-port`} inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, '').slice(0, 5))} placeholder="5173" autoComplete="off" />
      <label htmlFor={`${id}-hours`}>{t('preview-for')}</label>
      <select id={`${id}-hours`} value={hours} onChange={(e) => setHours(Number(e.target.value))}>
        {HOURS.map((h) => <option key={h} value={h}>{t('preview-hours', { hours: h })}</option>)}
      </select>
      <Button type="submit" variant="primary" disabled={!portOk || !node || busy}>{t('preview-open')}</Button>
    </form>
  );

  const suggestions = state.suggestions.length > 0 && (
    <ul className="preview-suggestions" aria-label={t('preview-suggestions')}>
      {[...state.suggestions].reverse().map((s) => (
        <li key={`${s.node}:${s.port}`} className="preview-suggestion">
          <span className="preview-suggestion__text">
            {s.agentId && agentName(s.agentId)
              ? t('preview-suggested', { agent: agentName(s.agentId), port: String(s.port), machine: machineName(s.node) })
              : t('preview-suggested-machine', { port: String(s.port), machine: machineName(s.node) })}
            {s.note && <span className="muted"> · {s.note}</span>}
          </span>
          <Button size="sm" disabled={busy} onClick={() => void open(s.node, s.port)}>{t('preview-open')}</Button>
        </li>
      ))}
    </ul>
  );

  const elsewhere = others.length > 0 && (
    <ul className="preview-others" aria-label={t('preview-others')}>
      {others.map((f) => (
        <li key={keyOf(f)} className="preview-other">
          <span>{t('preview-forward', { machine: machineName(f.node), port: String(f.port) })} · <span className="muted">{t('preview-other-device')}</span></span>
          <Button size="sm" variant="danger-ghost" onClick={() => void close(f)}>{t('preview-close')}</Button>
        </li>
      ))}
    </ul>
  );

  if (!current) {
    return (
      <div className="preview-pane preview-pane--empty">
        <p className="muted m-0">{t('preview-none')}</p>
        {error && <p role="alert" className="m-0 text-danger">{t(error)}</p>}
        {suggestions}
        {online.length > 0 ? form : <p className="muted m-0">{t('machines-empty')}</p>}
        {elsewhere}
      </div>
    );
  }

  return (
    <div className="preview-pane">
      <div className="preview-bar" role="toolbar" aria-label={t('preview-panel')}>
        {mine.length > 1 ? (
          <select aria-label={t('preview-which')} value={keyOf(current)} onChange={(e) => setChosen(e.target.value)}>
            {mine.map((f) => <option key={keyOf(f)} value={keyOf(f)}>{t('preview-forward', { machine: machineName(f.node), port: String(f.port) })}</option>)}
          </select>
        ) : <span className="preview-bar__name">{t('preview-forward', { machine: machineName(current.node), port: String(current.port) })}</span>}
        <form className="preview-address" onSubmit={(e) => { e.preventDefault(); go(); }}>
          <input aria-label={t('preview-address')} value={path} onChange={(e) => setPath(e.target.value)} spellCheck={false} autoComplete="off" />
        </form>
        <button type="button" className="ide-iconbtn" onClick={go} aria-label={t('preview-reload')} title={t('preview-reload')} disabled={!src}><Icon name="restore" /></button>
        <button type="button" className="ide-iconbtn" onClick={() => void newTab()} aria-label={t('preview-new-tab')} title={t('preview-new-tab')}><Icon name="maximize" /></button>
        <SegmentedControl label={t('preview-size')} value={size} onChange={setSize}
          items={[{ value: 'full', label: t('preview-size-full') }, { value: 'tablet', label: t('preview-size-tablet') }, { value: 'phone', label: t('preview-size-phone') }]} />
        {current.endsMs > 0 && <span className="preview-bar__until muted">{t('preview-until', { time: clock(current.endsMs) })}</span>}
        <button type="button" className="ide-iconbtn" onClick={() => setAdding((a) => !a)} aria-pressed={adding} aria-label={t('preview-open-title')} title={t('preview-open-title')}><Icon name="plus" /></button>
        <Button size="sm" variant="danger-ghost" onClick={() => void close(current)}>{t('preview-close')}</Button>
      </div>
      {error && <p role="alert" className="preview-error m-0 text-danger">{t(error)}</p>}
      <div className="preview-stage">
        {src && src.key === keyOf(current) ? (
          <iframe key={`${src.key}:${src.n}`} className="preview-frame" style={{ width: WIDTH[size] }} src={src.url}
            title={t('preview-forward', { machine: machineName(current.node), port: String(current.port) })}
            sandbox="allow-scripts allow-forms allow-same-origin allow-popups" referrerPolicy="no-referrer" />
        ) : <p className="muted m-0" role="status">{t('preview-entering')}</p>}
      </div>
      {(adding || suggestions || elsewhere) && <div className="preview-more">{suggestions}{elsewhere}{adding && form}</div>}
    </div>
  );
}
