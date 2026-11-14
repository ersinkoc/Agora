import { Fragment, useMemo, useRef, useState, type CSSProperties, type DragEvent, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { Icon } from '../components/Icon';
import { StateMark } from '../components/StateBadge';
import { NoAskBadge } from '../components/NoAskBadge';
import { type Action, type Place, type Workspace as WorkspaceState, displayRoot, visiblePanes } from './engine';
import { type Divider, type Frame, type Rect, arrange, dragDelta, dropPlace } from './geometry';
import { Menu, type MenuItem } from './Menu';
import { type Bindings, agentOf } from './streams';
import { type Pane, MIN_RATIO, bindOf, followOf, isKnownPanel, panesOf } from './tree';

const BAR = '2.25rem'; // the height of a pane's bar (h-9)
const DRAG_TYPE = 'application/x-agora-pane';
const pct = (v: number) => `${(v * 100).toFixed(4)}%`;
const edge = (v: number) => v < 0.9999;
// A frame's box: logical insets, so a row reads from the right in a right-to-left language. A 1px gap shows the line colour.
function box(r: Rect, top?: string): CSSProperties {
  return {
    position: 'absolute', insetInlineStart: pct(r.x), insetInlineEnd: pct(1 - r.x - r.w), top: top ? `calc(${pct(r.y)} + ${top})` : pct(r.y), bottom: pct(1 - r.y - r.h),
    marginInlineEnd: edge(r.x + r.w) ? 1 : 0, marginBottom: edge(r.y + r.h) ? 1 : 0,
  };
}

// The panel names the user sees for each panel id (UI vocabulary, not internal terms).
const PANEL_LABEL: Record<string, string> = {
  terminal: 'focus-terminal', conversation: 'chat-log', agent: 'layout-panel-agent', approvals: 'attention-title', changes: 'ide-tab-changes',
  files: 'console-files', tasks: 'board-title', messages: 'ide-tab-messages', usage: 'ide-tab-cost', theme: 'theme-editor-title', empty: 'layout-panel-empty',
};
export const panelLabel = (panel: string) => (PANEL_LABEL[panel] ? t(PANEL_LABEL[panel] as string) : t('layout-panel-unknown', { id: panel }));
const GLOBAL_PANELS = ['approvals', 'changes', 'tasks', 'messages', 'usage', 'theme'];

export interface WorkspaceProps {
  ws: WorkspaceState;
  dispatch: (a: Action) => void;
  fleet: readonly AgentTileModel[];
  bindings: Bindings;
  narrow: boolean;
  // The agent's own view (terminal, chat or closed summary) as the window draws it today.
  renderAgent: (a: AgentTileModel) => ReactNode;
  // A panel that is not about one agent (Needs you, Changes, ...); undefined when this window cannot draw it.
  renderPanel: (panel: string) => ReactNode | undefined;
  onFiles?: (a: AgentTileModel) => void;
  /** The agent's files drawn inside a Files pane; without it the pane offers to open them in a window. */
  renderFiles?: (a: AgentTileModel) => ReactNode | undefined;
  welcome: ReactNode;
  onStart?: () => void;
}

export function Workspace({ ws, dispatch, fleet, bindings, narrow, renderAgent, renderPanel, onFiles, renderFiles, welcome, onStart }: WorkspaceProps) {
  const host = useRef<HTMLDivElement>(null);
  const root = displayRoot(ws, narrow);
  const { frames, dividers } = useMemo(() => arrange(root), [root]);
  const visible = useMemo(() => new Set(visiblePanes(ws, narrow).map((p) => p.id)), [ws, narrow]);
  const all = panesOf(ws.layout.root);
  const byId = useMemo(() => new Map(fleet.map((a) => [a.id, a])), [fleet]);
  const agentFor = (p: Pane) => { const id = agentOf(p, bindings); return id ? byId.get(id) : undefined; };
  const label = (p: Pane) => { const a = agentFor(p); return a ? a.name : followOf(p) ? t(`layout-follow-${followOf(p)}`) : panelLabel(p.panel); };
  const [drag, setDrag] = useState<string | undefined>();
  const [drop, setDrop] = useState<{ frame: string; place: Place } | undefined>();
  const single = all.length === 1;
  const lone = single && all[0]?.panel === 'empty';
  const tiled = frames.length > 1;

  // Choosing what a pane shows: an agent, what needs you, the selection, or a panel.
  const showItems = (p: Pane): MenuItem[] => [
    { label: t('layout-follow-attention'), checked: followOf(p) === 'attention', run: () => dispatch({ type: 'bind', pane: p.id, bind: { follow: 'attention' }, panel: 'agent' }) },
    { label: t('layout-follow-selection'), checked: followOf(p) === 'selection', run: () => dispatch({ type: 'bind', pane: p.id, bind: { follow: 'selection' }, panel: 'agent' }) },
    ...fleet.filter((a) => !a.closed).map((a, i) => ({ label: t('layout-show-agent', { name: a.name }), divider: i === 0, checked: bindOf(p) === a.id, run: () => dispatch({ type: 'bind', pane: p.id, bind: { agent: a.id }, panel: 'agent' }) })),
    ...GLOBAL_PANELS.map((panel, i) => ({ label: panelLabel(panel), divider: i === 0, checked: p.panel === panel, run: () => dispatch({ type: 'bind', pane: p.id, panel }) })),
    // A pane bound to one agent shows the agent itself or its files (an editor's file tree next to its terminal).
    ...(bindOf(p) && renderFiles ? [
      { label: t('layout-show-agent-itself'), divider: true, checked: p.panel !== 'files', run: () => dispatch({ type: 'bind', pane: p.id, bind: { agent: bindOf(p) as string }, panel: 'agent' }) },
      { label: t('layout-show-agent-files'), checked: p.panel === 'files', run: () => dispatch({ type: 'bind', pane: p.id, bind: { agent: bindOf(p) as string }, panel: 'files' }) },
    ] : []),
  ];
  const others = (p: Pane) => all.filter((o) => o.id !== p.id);
  const places: [Place, string][] = [['start', 'layout-place-before'], ['end', 'layout-place-after'], ['top', 'layout-place-above'], ['bottom', 'layout-place-below'], ['center', 'layout-place-tab']];
  const paneItems = (p: Pane, f: Frame): MenuItem[] => [
    { label: t('layout-show'), items: showItems(p) },
    { label: t('layout-split-right'), divider: true, disabled: narrow, run: () => dispatch({ type: 'split', pane: p.id, direction: 'row' }) },
    { label: t('layout-split-down'), disabled: narrow, run: () => dispatch({ type: 'split', pane: p.id, direction: 'column' }) },
    ws.zoom ? { label: t('layout-restore'), run: () => dispatch({ type: 'restore' }) } : { label: t('layout-maximize'), disabled: single, run: () => dispatch({ type: 'maximize', pane: p.id }) },
    { label: t('layout-move-to'), disabled: others(p).length === 0, items: others(p).map((o) => ({ label: label(o), items: places.map(([place, key]) => ({ label: t(key), run: () => dispatch({ type: 'move', pane: p.id, target: o.id, place }) })) })) },
    { label: t('layout-swap-with'), disabled: others(p).length === 0, items: others(p).map((o) => ({ label: label(o), run: () => dispatch({ type: 'swap', a: p.id, b: o.id }) })) },
    ...(f.tabs && f.panes.length > 1 && !narrow ? [{ label: t('layout-move-out'), run: () => dispatch({ type: 'detach', pane: p.id }) }] : []),
    { label: t('layout-close-pane'), divider: true, run: () => dispatch({ type: 'close', pane: p.id }) },
  ];

  const tools = (p: Pane, f: Frame) => (
    <div className="flex flex-none items-center gap-0.5 px-1">
      {!narrow && !ws.zoom && <>
        <button type="button" className="ide-iconbtn" onClick={() => dispatch({ type: 'split', pane: p.id, direction: 'row' })} aria-label={t('layout-split-right')} title={t('layout-split-right')}><Icon name="splitRight" /></button>
        <button type="button" className="ide-iconbtn" onClick={() => dispatch({ type: 'split', pane: p.id, direction: 'column' })} aria-label={t('layout-split-down')} title={t('layout-split-down')}><Icon name="splitDown" /></button>
      </>}
      {ws.zoom ? <button type="button" className="ide-iconbtn" aria-pressed onClick={() => dispatch({ type: 'restore' })} aria-label={t('layout-restore')} title={t('layout-restore')}><Icon name="restore" /></button>
        : !single && <button type="button" className="ide-iconbtn" onClick={() => dispatch({ type: 'maximize', pane: p.id })} aria-label={t('layout-maximize')} title={t('layout-maximize')}><Icon name="maximize" /></button>}
      <Menu label={t('layout-pane-menu', { name: label(p) })} items={() => paneItems(p, f)}><Icon name="more" /></Menu>
    </div>
  );

  const dragProps = (p: Pane) => ({
    draggable: !narrow && !ws.zoom,
    onDragStart: (e: DragEvent) => {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData(DRAG_TYPE, p.id);
      e.dataTransfer.setData('text/plain', label(p));
      // The drop targets appear after the browser has taken its drag image (changing the page inside dragstart can cancel it).
      requestAnimationFrame(() => setDrag(p.id));
    },
    onDragEnd: () => { setDrag(undefined); setDrop(undefined); },
  });

  // The tab list of a group: arrows (mirrored right to left) and Home/End move, Delete or Backspace closes the focused tab; only the
  // tab in front is a stop for Tab, so the row is one stop on the way to the pane.
  const tabKey = (e: ReactKeyboardEvent<HTMLElement>, f: Frame, i: number) => {
    const ids = f.panes.map((x) => x.id);
    const rtl = getComputedStyle(e.currentTarget).direction === 'rtl';
    const stepBy = { ArrowRight: rtl ? -1 : 1, ArrowLeft: rtl ? 1 : -1 }[e.key];
    let to: number | undefined;
    if (e.key === 'Enter' || e.key === ' ') to = i;
    else if (stepBy !== undefined) to = (i + stepBy + ids.length) % ids.length;
    else if (e.key === 'Home') to = 0;
    else if (e.key === 'End') to = ids.length - 1;
    else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      const next = ids[i + 1] ?? ids[i - 1];
      dispatch({ type: 'close', pane: ids[i] as string });
      if (next) { dispatch({ type: 'activate', pane: next }); requestAnimationFrame(() => document.getElementById(`tab-${next}`)?.focus()); }
      return;
    } else return;
    e.preventDefault();
    const id = ids[to] as string;
    dispatch({ type: 'activate', pane: id });
    document.getElementById(`tab-${id}`)?.focus();
  };

  const bar = (f: Frame) => {
    const front = f.panes[f.active] as Pane;
    const focused = f.panes.some((p) => p.id === ws.focus);
    const a = agentFor(front);
    const ring = tiled && focused ? 'shadow-[inset_0_2px_0_var(--accent)]' : a?.state === 'needs-you' ? 'shadow-[inset_0_2px_0_var(--state-needs-you)]' : '';
    return (
      <div key={`bar:${f.key}`} style={{ ...box(f.rect), bottom: undefined, height: BAR }} className={`flex items-stretch border-b border-line bg-surface-1 ${ring}`}
        onPointerDownCapture={() => dispatch({ type: 'focus', pane: front.id })}>
        {f.tabs ? (
          <div role="tablist" aria-label={t('console-agents')} className="flex min-w-0 flex-1 overflow-x-auto quiet-scroll">
            {f.panes.map((p, i) => {
              const pa = agentFor(p);
              return (
                <div key={p.id} id={`tab-${p.id}`} role="tab" aria-selected={i === f.active} aria-controls={`pane-${p.id}`} aria-keyshortcuts="Delete" tabIndex={i === f.active ? 0 : -1}
                  className={`ide-tab group cursor-pointer ${drag && drag !== p.id ? 'ring-1 ring-inset ring-accent/50' : ''}`} title={t('layout-drag')}
                  onClick={() => dispatch({ type: 'activate', pane: p.id })} onKeyDown={(e) => tabKey(e, f, i)} {...dragProps(p)}>
                  {pa ? <StateMark state={pa.state} /> : <Icon name="layout" className="size-3.5 text-muted" />}
                  <span className="min-w-0 truncate">{label(p)}</span>
                  {pa && <NoAskBadge agent={pa} compact />}
                  <button type="button" tabIndex={-1} aria-hidden className="ide-iconbtn size-5 opacity-60 group-hover:opacity-100" onClick={(e) => { e.stopPropagation(); dispatch({ type: 'close', pane: p.id }); }} title={t('ide-close-tab', { name: label(p) })}><Icon name="close" className="size-3" /></button>
                </div>
              );
            })}
            {onStart && focused && <button type="button" className="ide-iconbtn m-1 flex-none" onClick={() => onStart()} aria-label={t('ide-new-tab')} title={t('ide-new-tab')}><Icon name="plus" /></button>}
          </div>
        ) : (
          <div className="flex min-w-0 flex-1 cursor-grab items-center gap-2 px-3 text-[0.8125rem]" title={t('layout-drag')} {...dragProps(front)}>
            {a ? <StateMark state={a.state} /> : <Icon name="layout" className="size-3.5 text-muted" />}
            <span className="min-w-0 truncate font-medium">{label(front)}</span>
            {a && <NoAskBadge agent={a} compact />}
            {a && <span className="hidden truncate text-xs text-muted sm:inline">{a.machine}</span>}
            {followOf(front) && <span className="ide-chip flex-none">{t(`layout-follows-${followOf(front)}`)}</span>}
          </div>
        )}
        {f.tabs && followOf(front) && <span className="ide-chip mx-1 flex-none self-center">{t(`layout-follows-${followOf(front)}`)}</span>}
        {tools(front, f)}
        {!f.tabs && !lone && <button type="button" className="ide-iconbtn me-1 self-center" onClick={() => dispatch({ type: 'close', pane: front.id })} aria-label={t('layout-close-pane')} title={t('layout-close-pane')}><Icon name="close" /></button>}
      </div>
    );
  };

  // What a pane shows. Hidden panes (background tabs, panes behind a maximized one) draw nothing, so they hold no stream.
  const content = (p: Pane): ReactNode => {
    const followed = followOf(p);
    const a = agentFor(p);
    if (p.panel === 'empty') return lone ? welcome : picker(p);
    if (!isKnownPanel(p.panel)) return <Note>{t('layout-panel-unknown', { id: p.panel })}</Note>;
    if (GLOBAL_PANELS.includes(p.panel)) return renderPanel(p.panel) ?? <Note>{t('layout-panel-unknown', { id: p.panel })}</Note>;
    if (!a) {
      if (followed === 'attention') return <Note>{t('attention-empty')}</Note>;
      if (followed === 'selection') return <Note>{t('layout-nothing-selected')}</Note>;
      if (bindOf(p)) return <Note>{t('layout-gone')}</Note>;
      return picker(p);
    }
    if (p.panel === 'files' && renderFiles) { const view = renderFiles(a); if (view) return <Fragment key={a.id}>{view}</Fragment>; }
    if (p.panel === 'files') return <Note>{onFiles ? <button type="button" className="button" onClick={() => onFiles(a)}><Icon name="files" />{t('layout-files-open', { name: a.name })}</button> : t('layout-files-none')}</Note>;
    if (p.panel === 'terminal' && a.terminal === false && !a.closed) return <Note>{t('layout-no-terminal')}</Note>;
    if (p.panel === 'conversation' && a.terminal !== false && !a.closed) return <Note>{t('layout-no-conversation')}</Note>;
    // Keyed by the agent: a pane that changes agent (following attention) mounts a new view; one that keeps it keeps its terminal.
    return <Fragment key={a.id}>{renderAgent(a)}</Fragment>;
  };

  const picker = (pane: Pane) => {
    const running = fleet.filter((a) => !a.closed).slice(0, 8);
    const pick = (b: Action) => dispatch(b);
    return (
      <div className="grid min-h-0 flex-1 place-items-center overflow-y-auto p-4">
        <div className="grid w-full max-w-xs gap-3">
          <h2 className="m-0">{t('layout-empty-title')}</h2>
          {running.length > 0 && <ul className="m-0 grid list-none gap-0.5 p-0">
            {running.map((a) => <li key={a.id}><button type="button" className="ide-row py-1.5" onClick={() => pick({ type: 'bind', pane: pane.id, bind: { agent: a.id }, panel: 'agent' })}><StateMark state={a.state} /><span className="min-w-0 flex-1 truncate">{a.name}</span><span className="truncate text-xs text-muted">{a.machine}</span></button></li>)}
          </ul>}
          <div className="flex flex-wrap gap-1.5">
            <button type="button" className="ide-chip hover:text-ink" onClick={() => pick({ type: 'bind', pane: pane.id, bind: { follow: 'attention' }, panel: 'agent' })}>{t('layout-follow-attention')}</button>
            <button type="button" className="ide-chip hover:text-ink" onClick={() => pick({ type: 'bind', pane: pane.id, bind: { follow: 'selection' }, panel: 'agent' })}>{t('layout-follow-selection')}</button>
            {GLOBAL_PANELS.map((panel) => <button key={panel} type="button" className="ide-chip hover:text-ink" onClick={() => pick({ type: 'bind', pane: pane.id, panel })}>{panelLabel(panel)}</button>)}
          </div>
        </div>
      </div>
    );
  };

  // Pane bodies, flat and in a fixed order (by id), so rearranging the tree moves boxes and never re-creates what is inside.
  const frameOf = new Map<string, Frame>();
  for (const f of frames) for (const p of f.panes) frameOf.set(p.id, f);
  const bodies = [...all].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)).map((p) => {
    const f = frameOf.get(p.id);
    if (!f || !visible.has(p.id)) return null;
    const a = agentFor(p);
    const name = label(p);
    return (
      <div key={p.id} id={`pane-${p.id}`} style={box(f.rect, BAR)} className="flex min-h-0 min-w-0 flex-col bg-surface-0"
        {...(f.tabs ? { role: 'tabpanel', 'aria-labelledby': `tab-${p.id}` } : { role: 'region', 'aria-label': t('layout-pane', { name }) })}
        data-pane={p.id} data-agent={a?.id} onPointerDownCapture={() => dispatch({ type: 'focus', pane: p.id })} onFocusCapture={() => dispatch({ type: 'focus', pane: p.id })}>
        {content(p)}
      </div>
    );
  });

  // While a pane is dragged, every frame is a drop target: an edge places it beside, the middle makes it a tab there.
  const overlays = drag ? frames.map((f) => {
    const target = f.panes[f.active] as Pane;
    const over = drop?.frame === f.key ? drop.place : undefined;
    const where = (e: DragEvent<HTMLDivElement>): Place => {
      const r = e.currentTarget.getBoundingClientRect();
      // Over a group's tab strip: in among its tabs.
      if (f.tabs && e.clientY - r.top < (e.currentTarget.firstElementChild?.getBoundingClientRect().height ?? 0)) return 'center';
      const rtl = getComputedStyle(e.currentTarget).direction === 'rtl';
      const x = (e.clientX - r.left) / Math.max(1, r.width);
      return dropPlace(rtl ? 1 - x : x, (e.clientY - r.top) / Math.max(1, r.height));
    };
    const zone: Record<Place, CSSProperties> = {
      center: { inset: '25%' }, start: { insetBlock: 0, insetInlineStart: 0, width: '50%' }, end: { insetBlock: 0, insetInlineEnd: 0, width: '50%' },
      top: { insetInline: 0, top: 0, height: '50%' }, bottom: { insetInline: 0, bottom: 0, height: '50%' },
    };
    return (
      <div key={`drop:${f.key}`} style={box(f.rect)} className="z-20" data-drop={target.id}
        onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = 'move'; const place = where(e); if (over !== place) setDrop({ frame: f.key, place }); }}
        onDragLeave={() => setDrop((cur) => (cur?.frame === f.key ? undefined : cur))}
        onDrop={(e) => {
          e.preventDefault();
          const id = e.dataTransfer.getData(DRAG_TYPE) || drag;
          // Dropped on a tab: it goes in that tab's place (reordering), else where the zone says.
          const tab = document.elementsFromPoint(e.clientX, e.clientY).find((el) => el.getAttribute('role') === 'tab' && el.id.startsWith('tab-'));
          dispatch({ type: 'move', pane: id, target: tab ? tab.id.slice(4) : target.id, place: tab ? 'center' : where(e) });
          setDrag(undefined); setDrop(undefined);
        }}>
        {f.tabs && <div aria-hidden className="pointer-events-none" style={{ height: BAR }} />}
        {over && <div aria-hidden className="pointer-events-none absolute rounded-xs border-2 border-accent bg-accent/15" style={zone[over]} />}
      </div>
    );
  }) : null;

  // Nothing open at all: the welcome, without a pane bar.
  if (lone) return <div className="flex min-h-0 flex-1 flex-col" data-layout-root>{welcome}</div>;

  return (
    <div ref={host} className="relative min-h-0 flex-1 overflow-hidden bg-line" data-layout-root>
      {frames.map(bar)}
      {bodies}
      {dividers.map((d) => <DividerLine key={`div:${d.path.join('.')}:${d.index}`} d={d} host={host} dispatch={dispatch} />)}
      {overlays}
    </div>
  );
}

function Note({ children }: { children: ReactNode }) {
  return <div className="grid min-h-0 flex-1 place-items-center p-4 text-center"><p className="muted m-0 max-w-xs">{children}</p></div>;
}

// The line between two panes: drag it, or focus it and use the arrows (Shift for bigger steps, Home/End for the limits).
function DividerLine({ d, host, dispatch }: { d: Divider; host: { current: HTMLDivElement | null }; dispatch: (a: Action) => void }) {
  const dragging = useRef(false);
  const row = d.direction === 'row';
  const at = d.start + d.ratio; // the line's place inside the split
  const min = Math.min(MIN_RATIO, (d.ratio + d.next) / 2);
  const pos = row ? { x: d.split.x + at * d.split.w, y: d.split.y } : { x: d.split.x, y: d.split.y + at * d.split.h };
  const style: CSSProperties = row
    ? { position: 'absolute', insetInlineStart: `calc(${pct(pos.x)} - 4px)`, top: pct(d.split.y), height: pct(d.split.h), width: 8 }
    : { position: 'absolute', top: `calc(${pct(pos.y)} - 4px)`, insetInlineStart: pct(d.split.x), width: pct(d.split.w), height: 8 };
  const resize = (delta: number) => dispatch({ type: 'resize', path: d.path, index: d.index, delta });
  const fromPointer = (clientX: number, clientY: number) => {
    const r = host.current?.getBoundingClientRect();
    if (!r) return;
    const rtl = !!host.current && getComputedStyle(host.current).direction === 'rtl';
    const fx = (clientX - r.left) / Math.max(1, r.width);
    const x = rtl ? 1 - fx : fx;
    const y = (clientY - r.top) / Math.max(1, r.height);
    const along = row ? (x - d.split.x) / Math.max(1e-6, d.split.w) : (y - d.split.y) / Math.max(1e-6, d.split.h);
    resize(dragDelta(d, along));
  };
  return (
    <div role="separator" tabIndex={0} aria-orientation={row ? 'vertical' : 'horizontal'} aria-label={t('layout-divider')}
      aria-valuenow={Math.round(at * 100)} aria-valuemin={Math.round((d.start + min) * 100)} aria-valuemax={Math.round((d.start + d.ratio + d.next - min) * 100)}
      style={style} className={`group z-10 touch-none outline-none ${row ? 'cursor-col-resize' : 'cursor-row-resize'}`}
      onPointerDown={(e) => { e.preventDefault(); e.currentTarget.setPointerCapture(e.pointerId); dragging.current = true; }}
      onPointerMove={(e) => { if (dragging.current) fromPointer(e.clientX, e.clientY); }}
      onPointerUp={() => { dragging.current = false; }} onPointerCancel={() => { dragging.current = false; }}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 0.1 : 0.02;
        const rtl = getComputedStyle(e.currentTarget).direction === 'rtl';
        const grow = row ? { ArrowRight: rtl ? -1 : 1, ArrowLeft: rtl ? 1 : -1 }[e.key] : { ArrowDown: 1, ArrowUp: -1 }[e.key];
        if (grow !== undefined) { e.preventDefault(); resize(grow * step); }
        else if (e.key === 'Home') { e.preventDefault(); resize(min - d.ratio); }
        else if (e.key === 'End') { e.preventDefault(); resize(d.ratio + d.next - min - d.ratio); }
      }}>
      <span aria-hidden className={`absolute bg-transparent group-hover:bg-accent/40 group-focus-visible:bg-accent group-active:bg-accent/60 ${row ? 'inset-y-0 start-[3px] w-0.5' : 'inset-x-0 top-[3px] h-0.5'}`} />
    </div>
  );
}
