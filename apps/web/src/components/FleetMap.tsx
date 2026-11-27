import { useEffect, useMemo, useRef, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { along, fleetMap, fromHub, route, taskSignals, type MapPoint } from '../net/fleetMap';
import type { BoardModel, MachineModel, MessageModel } from '../net/view';
import { visible } from '../net/view';

/** How long a moving item takes, and how many move at once (the oldest arrives at once beyond that). */
const FLIGHT_MS = 1100;
const MAX_FLIGHTS = 20;

interface Flight { key: string; path: MapPoint[]; start: number; words: string; card?: boolean }

const reduced = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
const messageKey = (m: MessageModel) => `${m.atMs}\u0000${m.fromId ?? m.from}\u0000${m.toId ?? m.to}\u0000${m.body.slice(0, 40)}`;

// ADR-0129: the live map of the fleet. The hub in the middle, the machines around it, each machine's agents beyond it, a helper further
// out from its parent; a mesh message travels as a dot from its sender to its recipient (through the hub between two machines). Clicking
// an agent opens it, a machine opens its page. Everything drawn is plain text from the view; nothing here changes state, and the frame
// loop runs only while something moves.
export function FleetMap({ machines, fleet, messages, boards = [], onOpen }: {
  machines: readonly MachineModel[]; fleet: readonly AgentTileModel[]; messages: readonly MessageModel[]; boards?: readonly BoardModel[]; onOpen: (agentId: string) => void;
}) {
  const map = useMemo(() => fleetMap(machines, fleet), [machines, fleet]);
  const [flights, setFlights] = useState<Flight[]>([]);
  const [now, setNow] = useState(() => performance.now());
  const [flash, setFlash] = useState<string | undefined>();
  const seen = useRef<Set<string> | undefined>(undefined);
  const seenTasks = useRef<Set<string> | undefined>(undefined);
  // The pane's size in pixels: the picture is fitted to it, and text and marks keep the same size on screen whatever it is.
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 600, h: 600 });
  useEffect(() => {
    const el = box.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(([e]) => { if (e) setSize({ w: Math.max(120, e.contentRect.width), h: Math.max(120, e.contentRect.height) }); });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // New messages since the map opened become flights (the ones already there when it opened do not move).
  useEffect(() => {
    const keys = messages.map(messageKey);
    if (!seen.current) { seen.current = new Set(keys); return; }
    const fresh = messages.filter((m) => !seen.current?.has(messageKey(m)));
    for (const k of keys) seen.current.add(k);
    if (fresh.length === 0) return;
    if (reduced()) { setFlash(fresh[0]?.toId); return; }
    const at = performance.now();
    const add: Flight[] = [];
    for (const m of fresh) {
      const path = m.fromId && m.toId ? route(map, m.fromId, m.toId) : undefined;
      if (path) add.push({ key: messageKey(m), path, start: at, words: t('map-message', { from: visible(m.from), to: visible(m.to), text: visible(m.body).slice(0, 120) }) });
    }
    if (add.length) setFlights((f) => [...f, ...add].slice(-MAX_FLIGHTS));
  }, [messages, map]);

  // A task given to an agent and a handoff delivered to one, since the map opened, move as a card from the hub.
  const signals = useMemo(() => taskSignals(boards), [boards]);
  useEffect(() => {
    if (!seenTasks.current) { seenTasks.current = new Set(signals.map((s) => s.key)); return; }
    const fresh = signals.filter((s) => !seenTasks.current?.has(s.key));
    for (const s of fresh) seenTasks.current.add(s.key);
    if (fresh.length === 0) return;
    if (reduced()) { setFlash(map.agents.find((a) => a.id === fresh[0]?.to || a.id.endsWith(`/${fresh[0]?.to}`))?.id); return; }
    const at = performance.now();
    const add: Flight[] = [];
    for (const s of fresh) {
      const path = fromHub(map, s.to);
      if (path) add.push({ key: s.key, path, start: at, words: t('map-task', { text: visible(s.words).slice(0, 120) }), card: true });
    }
    if (add.length) setFlights((f) => [...f, ...add].slice(-MAX_FLIGHTS));
  }, [signals, map]);

  // The frame loop: only while something moves.
  useEffect(() => {
    if (flights.length === 0) return;
    let raf = 0;
    const tick = () => {
      const at = performance.now();
      setNow(at);
      setFlights((f) => {
        const left = f.filter((x) => at - x.start < FLIGHT_MS);
        return left.length === f.length ? f : left;
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [flights.length]);

  useEffect(() => {
    if (!flash) return;
    const timer = window.setTimeout(() => setFlash(undefined), 900);
    return () => window.clearTimeout(timer);
  }, [flash]);

  // The picture's extent, fitted to the pane with room for the labels; `u` is how many picture units one screen pixel is.
  const xs = [0, ...map.machines.map((m) => m.x), ...map.agents.map((a) => a.x)];
  const ys = [0, ...map.machines.map((m) => m.y), ...map.agents.map((a) => a.y)];
  const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const margin = 48; // pixels kept around the picture for marks and labels
  const u = Math.max((maxX - minX || 1) / Math.max(40, size.w - 2 * margin), (maxY - minY || 1) / Math.max(40, size.h - 2 * margin), 0.05);
  const pad = margin * u;
  const view = `${minX - pad} ${minY - pad} ${maxX - minX + 2 * pad} ${maxY - minY + 2 * pad}`;
  const px = (n: number) => n * u;
  if (machines.length === 0) return <div className="fleet-map fleet-map--empty"><p className="muted m-0">{t('machines-empty')}</p></div>;
  return (
    <div className="fleet-map" ref={box}>
      <svg viewBox={view} style={{ fontSize: px(12.5), strokeWidth: px(1.5) }} role="img" aria-label={t('map-label', { machines: machines.length, agents: map.agents.length })} preserveAspectRatio="xMidYMid meet">
        <g className="fleet-map__edges">
          {map.edges.map((e, i) => (
            <line key={i} x1={e.from.x} y1={e.from.y} x2={e.to.x} y2={e.to.y} className={`fleet-map__edge fleet-map__edge--${e.kind}${e.dim ? ' fleet-map__edge--dim' : ''}`} />
          ))}
        </g>
        <g className="fleet-map__hub">
          <circle cx={0} cy={0} r={px(20)} />
          <text x={0} y={px(4)} textAnchor="middle">{t('map-hub')}</text>
          <text x={0} y={px(36)} textAnchor="middle" className="fleet-map__note">{t('map-you')}</text>
        </g>
        {map.machines.map((m) => (
          <a key={m.id} href={`#/machines/${encodeURIComponent(m.id)}`} className={`fleet-map__machine${m.online ? '' : ' fleet-map__machine--off'}`}>
            <title>{m.online ? visible(m.name) : t('map-machine-offline', { name: visible(m.name) })}</title>
            <rect x={m.x - px(13)} y={m.y - px(13)} width={px(26)} height={px(26)} rx={px(6)} />
            <text x={m.x} y={m.y + px(28)} textAnchor="middle">{visible(m.name).slice(0, 20)}</text>
            {m.hidden > 0 && <text x={m.x} y={m.y + px(4)} textAnchor="middle" className="fleet-map__more">+{m.hidden}</text>}
          </a>
        ))}
        {map.agents.map((a) => (
          <g key={a.id} className={`fleet-map__agent fleet-map__agent--${a.state}${flash === a.id ? ' fleet-map__agent--flash' : ''}`} role="button" tabIndex={0}
            aria-label={t('map-agent', { name: visible(a.name), state: t(`state-${a.state}`) })}
            onClick={() => onOpen(a.id)} onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onOpen(a.id); } }}>
            <title>{`${visible(a.name)} · ${t(`state-${a.state}`)}`}</title>
            <circle cx={a.x} cy={a.y} r={px(8)} style={{ fill: a.accent }} />
            <circle cx={a.x} cy={a.y} r={px(11.5)} className="fleet-map__ring" style={{ strokeWidth: px(2.5) }} />
          </g>
        ))}
        <g className="fleet-map__flights" aria-hidden>
          {flights.map((f) => {
            const p = along(f.path, (now - f.start) / FLIGHT_MS);
            return f.card
              ? <rect key={f.key} x={p.x - px(6)} y={p.y - px(4.5)} width={px(12)} height={px(9)} rx={px(1.5)} className="fleet-map__card"><title>{f.words}</title></rect>
              : <circle key={f.key} cx={p.x} cy={p.y} r={px(4.5)} className="fleet-map__dot"><title>{f.words}</title></circle>;
          })}
        </g>
      </svg>
    </div>
  );
}
