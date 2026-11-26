import { Fragment, useMemo, useRef, useState, type CSSProperties, type DragEvent, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { t } from '../i18n';
import { agentKind, endedWithError, type AgentTileModel } from '../model';
import { Icon } from '../components/Icon';
import { AgentAvatar, StatusMark } from '../components/kit';
import { StateMark } from '../components/StateBadge';
import { NoAskBadge } from '../components/NoAskBadge';
import { FinishedMark } from '../components/FinishedMark';
import { type Action, type Place, type Workspace as WorkspaceState, displayRoot, visiblePanes } from './engine';
import { type Divider, type Frame, type Rect, arrange, dragDelta, dropPlace } from './geometry';
import { Menu, type MenuItem } from './Menu';
import { TabStrip } from './TabStrip';
import { type Bindings, agentOf } from './streams';
import { type Pane, MIN_RATIO, bindOf, followOf, panesOf } from './tree';
import { Note } from './Note';
import { SAFE, safeAddress } from '../plugins/safe';
import { PANEL_INFO, type PanelActions, type PanelContext, type PanelDef, type PanelInfo, type PanelRegistry, type PanelSources, type PanelState } from './panels';
import type { TerminalSource } from '../terminal/source';

// The height of a pane's bar (a theme's density sets it, app.css), the space between panes (a theme's gaps and lines), and whether the
// bars are at the bottom of their panes (1) or the top (0): CSS variables the theme sets, so the layout follows it with no state of its own.
const BAR = 'var(--pane-bar-height, 2.25rem)';
const SPLIT = 'var(--pane-split, 1px)';
const AT_END = 'var(--tabs-at-end, 0)';
const DRAG_TYPE = 'application/x-agora-pane';
const pct = (v: number) => `${(v * 100).toFixed(4)}%`;
const edge = (v: number) => v < 0.9999;
// A frame's box: logical insets, so a row reads from the right in a right-to-left language. The space between frames (`--pane-split`,
// 1px by default) shows the layout's background: the line colour, or the background when a theme sets gaps.
function box(r: Rect): CSSProperties {
  return {
    position: 'absolute', insetInlineStart: pct(r.x), insetInlineEnd: pct(1 - r.x - r.w), top: pct(r.y), bottom: pct(1 - r.y - r.h),
    marginInlineEnd: edge(r.x + r.w) ? SPLIT : 0, marginBottom: edge(r.y + r.h) ? SPLIT : 0,
  };
}
// A pane's body: the frame without its bar, which is at the top, or at the bottom when a theme says so.
function bodyBox(r: Rect): CSSProperties {
  return { ...box(r), top: `calc(${pct(r.y)} + (1 - ${AT_END}) * ${BAR})`, bottom: `calc(${pct(1 - r.y - r.h)} + ${AT_END} * ${BAR})` };
}
// A frame's bar: its height, at the top of the frame or (with the bars at the bottom) just above the space below it.
function barBox(r: Rect): CSSProperties {
  const below = edge(r.y + r.h) ? SPLIT : '0px';
  return { ...box(r), top: `calc(${pct(r.y)} + ${AT_END} * (${pct(r.h)} - ${BAR} - ${below}))`, bottom: undefined, marginBottom: 0, height: BAR };
}

// The name the user sees for a panel id (UI vocabulary, not internal terms), from the registry; an id it does not have is named as unknown.
// A plugin's panel is named by the plugin's own name as plain text, never looked up as a Fluent key (ADR-0114; the marker says whose it is).
export const panelName = (panels: Pick<PanelRegistry<PanelInfo>, 'get'>, panel: string) => {
  const def = panels.get(panel);
  if (def?.plugin) return def.plugin.name;
  if (def) return t(def.label);
  return isPluginPanel(panel) ? t('layout-panel-plugin') : t('layout-panel-unknown', { id: panel });
};

// A plugin's panel this window does not have (safe mode, or a plugin no longer kept here) is named in plain words, never by its id
// (`plugin.<id>.<panel>` is an internal name; visual-language.md, "UI vocabulary").
const isPluginPanel = (panel: string) => panel.startsWith('plugin.');
const missingNote = (panel: string) =>
  (isPluginPanel(panel) ? t(SAFE ? 'layout-panel-plugin-safe' : 'layout-panel-plugin-missing') : t('layout-panel-unknown', { id: panel }));

// The "plugin" marker the window draws beside a plugin panel's name, in its pane bar and its tab (ADR-0114 decision 4): the window's own
// words, which no plugin can reach and no theme can restyle (the ADR-0113 floor, app.css `.plugin-mark`); "development" too for a plugin
// loaded from a local file.
export function PluginMark({ dev }: { dev: boolean }) {
  return (
    <span className="plugin-mark" data-plugin-mark="" title={dev ? `${t('plugin-marker')} · ${t('plugin-development')}` : t('plugin-marker')}>
      <Icon name="puzzle" className="size-3" />{t('plugin-marker')}{dev && <>{' '}<span className="plugin-mark__dev">{t('plugin-development')}</span></>}
    </span>
  );
}
export const panelLabel = (panel: string) => panelName(PANEL_INFO, panel);

export interface WorkspaceProps {
  ws: WorkspaceState;
  dispatch: (a: Action) => void;
  fleet: readonly AgentTileModel[];
  bindings: Bindings;
  narrow: boolean;
  /** The panels this window can draw (ADR-0068 rung 3): a pane's panel id is looked up here; one it does not have is "unknown". */
  panels: PanelRegistry;
  /** What each panel may see and do (its context, besides its pane and binding). The window keeps the connection and the keys. */
  state: PanelState;
  sources: PanelSources;
  actions: Omit<PanelActions, 'rebind'>;
  /** The terminal of an agent, given only to a panel that streams and only while its pane is drawn. */
  terminalFor: (agentId: string) => TerminalSource;
  welcome: ReactNode;
  /** What the + button of a tab group offers (start an agent, a terminal, the tools beside the agent in focus). */
  plusItems?: () => MenuItem[];
}

export function Workspace({ ws, dispatch, fleet, bindings, narrow, panels, state, sources, actions, terminalFor, welcome, plusItems }: WorkspaceProps) {
  const host = useRef<HTMLDivElement>(null);
  const root = displayRoot(ws, narrow);
  const { frames, dividers } = useMemo(() => arrange(root), [root]);
  const visible = useMemo(() => new Set(visiblePanes(ws, narrow).map((p) => p.id)), [ws, narrow]);
  const all = panesOf(ws.layout.root);
  const byId = useMemo(() => new Map(fleet.map((a) => [a.id, a])), [fleet]);
  const agentFor = (p: Pane) => { const id = agentOf(p, bindings); return id ? byId.get(id) : undefined; };
  // A pane's name: the agent it shows; for a tool (Terminal, Changes, Files, File manager) the tool first, so two panes of one agent differ.
  const panelLabel = (panel: string) => panelName(panels, panel);
  const label = (p: Pane) => { const a = agentFor(p); return a ? (panels.get(p.panel)?.kind === 'tool' ? t('layout-tool-of', { what: panelLabel(p.panel), name: a.name }) : a.name) : followOf(p) ? t(`layout-follow-${followOf(p)}`) : panelLabel(p.panel); };
  const [drag, setDrag] = useState<string | undefined>();
  const [drop, setDrop] = useState<{ frame: string; place: Place } | undefined>();
  const single = all.length === 1;
  // A window with only Home (the fleet) or nothing in it shows Home without a pane bar (the registry's `lone`); any other panel alone
  // keeps the bar the window draws.
  const lone = single && !!panels.get(all[0]?.panel ?? '')?.lone;
  const globals = panels.offered('global');
  const tools = panels.offered('tool');
  const tiled = frames.length > 1;

  // Choosing what a pane shows: an agent, what needs you, the selection, or a panel.
  const showItems = (p: Pane): MenuItem[] => [
    { label: t('layout-follow-attention'), checked: followOf(p) === 'attention', run: () => dispatch({ type: 'bind', pane: p.id, bind: { follow: 'attention' }, panel: 'agent' }) },
    { label: t('layout-follow-selection'), checked: followOf(p) === 'selection', run: () => dispatch({ type: 'bind', pane: p.id, bind: { follow: 'selection' }, panel: 'agent' }) },
    ...fleet.filter((a) => !a.closed).map((a, i) => ({ label: t('layout-show-agent', { name: a.name }), divider: i === 0, checked: bindOf(p) === a.id, run: () => dispatch({ type: 'bind', pane: p.id, bind: { agent: a.id }, panel: 'agent' }) })),
    ...globals.map(({ id: panel }, i) => ({ label: panelLabel(panel), divider: i === 0, checked: p.panel === panel, run: () => dispatch({ type: 'bind', pane: p.id, panel }) })),
    // The tools: for a pane bound to one agent, the same agent's terminal, changes, files and file manager (an editor's file tree next to
    // its terminal); for any other, the tool's own chooser.
    ...(tools.length > 0 ? [
      ...(bindOf(p) ? [{ label: t('layout-show-agent-itself'), divider: true, checked: p.panel === 'agent', run: () => dispatch({ type: 'bind', pane: p.id, bind: { agent: bindOf(p) as string }, panel: 'agent' }) }] : []),
      ...tools.map(({ id: panel }, i) => ({ label: panelLabel(panel), divider: i === 0 && !bindOf(p), checked: p.panel === panel,
        run: () => dispatch({ type: 'bind', pane: p.id, panel, ...(bindOf(p) ? { bind: { agent: bindOf(p) as string } } : {}) }) })),
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
    // A plugin's pane offers the way out by the mouse too: the window again with no plugins and the built-in theme (ADR-0114).
    ...(panels.get(p.panel)?.plugin ? [{ label: t('safe-mode-open'), divider: true, run: () => location.assign(safeAddress(location.href)) }] : []),
    { label: t('layout-close-pane'), divider: true, run: () => dispatch({ type: 'close', pane: p.id }) },
  ];

  // Split and maximize are also in the ⋯ menu, so a narrow pane folds them away first (app.css `.pane-bar`) and keeps its name.
  const barTools = (p: Pane, f: Frame) => (
    <div className="flex flex-none items-center gap-0.5 px-1">
      {!narrow && !ws.zoom && <>
        <button type="button" data-pane-tool="split" className="ide-iconbtn" onClick={() => dispatch({ type: 'split', pane: p.id, direction: 'row' })} aria-label={t('layout-split-right')} title={t('layout-split-right')}><Icon name="splitRight" /></button>
        <button type="button" data-pane-tool="split" className="ide-iconbtn" onClick={() => dispatch({ type: 'split', pane: p.id, direction: 'column' })} aria-label={t('layout-split-down')} title={t('layout-split-down')}><Icon name="splitDown" /></button>
      </>}
      {ws.zoom ? <button type="button" className="ide-iconbtn" aria-pressed onClick={() => dispatch({ type: 'restore' })} aria-label={t('layout-restore')} title={t('layout-restore')}><Icon name="restore" /></button>
        : !single && <button type="button" data-pane-tool="maximize" className="ide-iconbtn" onClick={() => dispatch({ type: 'maximize', pane: p.id })} aria-label={t('layout-maximize')} title={t('layout-maximize')}><Icon name="maximize" /></button>}
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
      <div key={`bar:${f.key}`} style={barBox(f.rect)} className={`pane-bar flex items-stretch border-b border-panebar-line bg-panebar text-panebar-ink ${ring}`}
        onPointerDownCapture={() => dispatch({ type: 'focus', pane: front.id })}>
        {f.tabs ? (
          <TabStrip tabs={f.panes.map((p, i) => { const pa = agentFor(p); return { id: p.id, label: label(p), active: i === f.active, state: pa?.state, failed: pa ? endedWithError(pa) : undefined }; })} onPick={(id) => dispatch({ type: 'activate', pane: id })}
            trailing={plusItems && focused ? <Menu label={t('ide-new-tab')} items={plusItems} className="ide-iconbtn m-1 flex-none self-center"><Icon name="plus" /></Menu> : undefined}>
            {f.panes.map((p, i) => {
              const pa = agentFor(p);
              return (
                <div key={p.id} id={`tab-${p.id}`} role="tab" aria-selected={i === f.active} aria-controls={`pane-${p.id}`} aria-keyshortcuts="Delete" tabIndex={i === f.active ? 0 : -1}
                  className={`ide-tab group cursor-pointer ${drag && drag !== p.id ? 'ring-1 ring-inset ring-accent/50' : ''}`} title={label(p)}
                  onClick={() => dispatch({ type: 'activate', pane: p.id })} onKeyDown={(e) => tabKey(e, f, i)} {...dragProps(p)}>
                  {pa ? <AgentAvatar accent={pa.accent} kind={agentKind(pa)} size="s" /> : <Icon name="layout" className="size-3.5 text-muted" />}
                  <PaneName text={label(p)} />
                  {panels.get(p.panel)?.plugin && <PluginMark dev={!!panels.get(p.panel)?.plugin?.dev} />}
                  {pa && <NoAskBadge agent={pa} nested bare />}
                  {pa && <FinishedMark agentId={pa.id} compact />}
                  {pa && <StatusMark state={pa.state} failed={endedWithError(pa)} />}
                  <button type="button" tabIndex={-1} aria-hidden className="ide-iconbtn size-5 opacity-60 group-hover:opacity-100" onClick={(e) => { e.stopPropagation(); dispatch({ type: 'close', pane: p.id }); }} title={t('ide-close-tab', { name: label(p) })}><Icon name="close" className="size-3" /></button>
                </div>
              );
            })}
          </TabStrip>
        ) : (
          <div className="@container flex min-w-0 flex-1 cursor-grab items-center gap-2 px-3 text-[0.8125rem]" title={t('layout-drag')} data-pane-label="" {...dragProps(front)}>
            {a ? <span className="flex flex-none" title={label(front)}><AgentAvatar accent={a.accent} kind={agentKind(a)} size="s" /></span> : <Icon name="layout" className="size-3.5 flex-none text-muted" />}
            <PaneName text={label(front)} className="font-medium" />
            {a && <NoAskBadge agent={a} bare />}
            {a && <FinishedMark agentId={a.id} compact />}
            {a && <StatusMark state={a.state} failed={endedWithError(a)} />}
            {a && <span className="hidden flex-none text-xs text-muted @lg:inline">{a.machine}</span>}
            {followOf(front) && <span className="ide-chip flex-none">{t(`layout-follows-${followOf(front)}`)}</span>}
          </div>
        )}
        {f.tabs && followOf(front) && <span className="ide-chip mx-1 flex-none self-center">{t(`layout-follows-${followOf(front)}`)}</span>}
        {(() => {
          const tools = <>
            {barTools(front, f)}
            {!f.tabs && !lone && <button type="button" className="ide-iconbtn me-1 self-center" onClick={() => dispatch({ type: 'close', pane: front.id })} aria-label={t('layout-close-pane')} title={t('layout-close-pane')}><Icon name="close" /></button>}
          </>;
          // A plugin's pane: the marker keeps its whole width in any pane; the name gives way first, then the pane's own buttons, which are
          // cut at the end rather than drawn over it (they are all in the ⋯ menu, and the pane can be widened).
          const plugin = !f.tabs ? panels.get(front.panel)?.plugin : undefined;
          return plugin ? <>
            <span className="flex flex-none items-center pe-1"><PluginMark dev={plugin.dev} /></span>
            <div className="flex min-w-0 items-stretch overflow-hidden" data-pane-tools="">{tools}</div>
          </> : tools;
        })()}
      </div>
    );
  };

  // What a pane's panel is given: its pane, its binding (for a panel bound to an agent), the window's read-only state and its actions,
  // with `rebind` for this pane, and a terminal only for a panel that streams. Frozen at the top level; `readonly` types keep built-ins from changing what is inside.
  const contextOf = (p: Pane, def: PanelDef, a: AgentTileModel | undefined): PanelContext => {
    const bound = def.binds === 'agent';
    const agent = bound ? a : undefined;
    const rebind = (id: string) => dispatch({ type: 'bind', pane: p.id, bind: { agent: id }, panel: p.panel });
    return Object.freeze({
      pane: p.id, panel: p.panel, bind: bound ? p.bind : undefined, agent, state, sources,
      actions: Object.freeze({ ...actions, rebind }),
      terminal: def.streams && agent ? () => terminalFor(agent.id) : undefined,
    });
  };

  // What a pane shows: its panel from the registry, under the bar this window draws. Hidden panes (background tabs, panes behind a
  // maximized one) draw nothing, so they hold no stream. The window resolves a panel's binding: with no agent to show it says why,
  // and an agent's view bound to nothing shows the chooser (a tool draws its own).
  const content = (p: Pane): ReactNode => {
    const followed = followOf(p);
    const a = agentFor(p);
    const def = panels.get(p.panel);
    const unknown = <Note>{missingNote(p.panel)}</Note>;
    if (!def) return unknown;
    if (def.kind === 'chooser') return lone ? welcome : picker(p);
    if (def.binds === 'agent' && !a) {
      if (followed === 'attention') return <Note>{t('attention-empty')}</Note>;
      if (followed === 'selection') return <Note>{t('layout-nothing-selected')}</Note>;
      if (bindOf(p)) return <Note>{t('layout-gone')}</Note>;
      if (def.kind !== 'tool') return picker(p);
    }
    return def.render(contextOf(p, def, a)) ?? unknown;
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
            {[...tools, ...globals].map(({ id: panel }) => <button key={panel} type="button" className="ide-chip hover:text-ink" onClick={() => pick({ type: 'bind', pane: pane.id, panel })}>{panelLabel(panel)}</button>)}
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
      <div key={p.id} id={`pane-${p.id}`} style={bodyBox(f.rect)} className="flex min-h-0 min-w-0 flex-col bg-surface-0"
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
      // Over a group's tab strip (at the top, or at the bottom when a theme puts it there): in among its tabs.
      const strip = e.currentTarget.firstElementChild?.getBoundingClientRect().height ?? 0;
      const atEnd = document.documentElement.dataset.tabBar === 'bottom';
      if (f.tabs && (atEnd ? r.bottom - e.clientY : e.clientY - r.top) < strip) return 'center';
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
    <div ref={host} className="relative min-h-0 flex-1 overflow-hidden" data-layout-root data-tiled>
      {frames.map(bar)}
      {bodies}
      {dividers.map((d) => <DividerLine key={`div:${d.path.join('.')}:${d.index}`} d={d} host={host} dispatch={dispatch} />)}
      {overlays}
    </div>
  );
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

// A pane's or tab's name gives way from the front, never its last part ("cl… · teal", "Changes · cl… · amber"): the avatar already shows
// the kind's letter, and the colour word is what tells two agents of one kind apart. A name without " · " ends in an ellipsis as usual.
function PaneName({ text, className = '' }: { text: string; className?: string }) {
  const cut = text.lastIndexOf(' · ');
  if (cut <= 0) return <span className={`min-w-0 truncate ${className}`} data-pane-name>{text}</span>;
  return (
    <span className={`truncate flex min-w-0 ${className}`} data-pane-name>
      <span className="min-w-[1.5ch] truncate">{text.slice(0, cut)}</span><span className="flex-none whitespace-pre">{text.slice(cut)}</span>
    </span>
  );
}
