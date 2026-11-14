// The layout engine: one pure reducer over the workspace (the tree, the pane in focus, the maximized pane). Every action returns a
// valid tree; one that would break a rule (too many panes, too deep, an unknown pane) returns the workspace unchanged.
import {
  type Bind, type Layout, type LayoutNode, type Pane, type Split, type Tabs,
  LAYOUT_VERSION, MAX_DEPTH, MAX_PANES, MIN_RATIO, bindOf, depthOf, emptyPane, freshId, normalise, panesOf,
} from './tree';

export interface Workspace {
  layout: Layout;
  focus?: string; // the pane that keys and "open" go to
  zoom?: string; // the maximized pane, drawn alone until restored
}

export const PRESETS = ['1x1', '2x1', '1x2', '2x2', '3x2', 'focus'] as const;
export type Preset = (typeof PRESETS)[number];
// Logical edges: `start`/`end` follow the reading direction (left/right in English, mirrored right to left).
export type Place = 'center' | 'start' | 'end' | 'top' | 'bottom';

export type Action =
  | { type: 'split'; pane: string; direction: 'row' | 'column'; panel?: string; bind?: Bind }
  | { type: 'close'; pane: string }
  | { type: 'resize'; path: readonly number[]; index: number; delta: number }
  | { type: 'swap'; a: string; b: string }
  | { type: 'move'; pane: string; target: string; place: Place }
  | { type: 'detach'; pane: string }
  | { type: 'activate'; pane: string }
  | { type: 'focus'; pane: string }
  | { type: 'maximize'; pane: string }
  | { type: 'restore' }
  | { type: 'preset'; preset: Preset; agents: readonly string[] }
  | { type: 'bind'; pane: string; bind?: Bind; panel?: string }
  | { type: 'open'; agent: string; background?: boolean }
  | { type: 'show'; panel: string }
  | { type: 'prune'; alive: ReadonlySet<string> }
  | { type: 'replace'; layout: Layout };

type Path = number[];

export const workspaceOf = (layout: Layout): Workspace => fix({ layout });

function at(node: LayoutNode, path: readonly number[]): LayoutNode | undefined {
  let n: LayoutNode | undefined = node;
  for (const i of path) n = n && n.type !== 'pane' ? (n.children as LayoutNode[])[i] : undefined;
  return n;
}

function replaceAt(node: LayoutNode, path: readonly number[], next: LayoutNode): LayoutNode {
  if (path.length === 0) return next;
  const [i, ...rest] = path as [number, ...number[]];
  if (node.type === 'pane') return node;
  if (node.type === 'tabs') return next.type === 'pane' && rest.length === 0 ? { ...node, children: node.children.map((c, j) => (j === i ? next : c)) } : node;
  return { ...node, children: node.children.map((c, j) => (j === i ? replaceAt(c, rest, next) : c)) };
}

export function pathOf(node: LayoutNode, id: string, prefix: Path = []): Path | undefined {
  if (node.type === 'pane') return node.id === id ? prefix : undefined;
  for (let i = 0; i < node.children.length; i++) {
    const p = pathOf(node.children[i] as LayoutNode, id, [...prefix, i]);
    if (p) return p;
  }
  return undefined;
}

export const findPane = (layout: Layout, id: string | undefined): Pane | undefined => (id ? panesOf(layout.root).find((p) => p.id === id) : undefined);

// The tab group a pane sits in, if any: its path and the pane's index in it.
function groupOf(root: LayoutNode, id: string): { path: Path; index: number; tabs: Tabs } | undefined {
  const p = pathOf(root, id);
  if (!p || p.length === 0) return undefined;
  const parent = at(root, p.slice(0, -1));
  return parent?.type === 'tabs' ? { path: p.slice(0, -1), index: p[p.length - 1] as number, tabs: parent } : undefined;
}

// The tabs of the group a pane sits in (undefined for a pane on its own).
export const tabsOf = (layout: Layout, id: string | undefined): Pane[] | undefined => (id ? groupOf(layout.root, id)?.tabs.children : undefined);

// Removes a pane. Its tab group closes when it was the last tab; in a split its siblings take its space in proportion.
function remove(root: LayoutNode, id: string): LayoutNode | undefined {
  const walk = (n: LayoutNode): LayoutNode | undefined => {
    if (n.type === 'pane') return n.id === id ? undefined : n;
    if (n.type === 'tabs') {
      const i = n.children.findIndex((c) => c.id === id);
      if (i < 0) return n;
      const children = n.children.filter((_, j) => j !== i);
      if (children.length === 0) return undefined;
      return { type: 'tabs', children, active: n.active > i || n.active === children.length ? n.active - 1 : n.active };
    }
    const children: LayoutNode[] = [];
    const ratios: number[] = [];
    n.children.forEach((c, j) => { const k = walk(c); if (k) { children.push(k); ratios.push(n.ratios[j] as number); } });
    return normalise({ ...n, children, ratios });
  };
  return walk(root);
}

// Puts `node` beside the pane `target` (beside its whole tab group when it is a tab).
function insertBeside(root: LayoutNode, target: string, node: LayoutNode, direction: 'row' | 'column', after: boolean): LayoutNode {
  let unit = pathOf(root, target);
  if (!unit) return root;
  if (groupOf(root, target)) unit = unit.slice(0, -1);
  const parentPath = unit.slice(0, -1);
  const parent = unit.length > 0 ? at(root, parentPath) : undefined;
  if (parent?.type === 'split' && parent.direction === direction) {
    const i = unit[unit.length - 1] as number;
    const share = (parent.ratios[i] as number) / 2;
    const children = [...parent.children];
    const ratios = [...parent.ratios];
    children.splice(after ? i + 1 : i, 0, node);
    ratios.splice(i, 1, share, share);
    return replaceAt(root, parentPath, { ...parent, children, ratios });
  }
  const here = at(root, unit) as LayoutNode;
  const split: Split = { type: 'split', direction, ratios: [0.5, 0.5], children: after ? [here, node] : [node, here] };
  return replaceAt(root, unit, split);
}

// Adds `pane` as a tab next to `target`, turning a lone pane into a tab group.
function addTab(root: LayoutNode, target: string, pane: Pane): LayoutNode {
  const g = groupOf(root, target);
  if (g) {
    const children = [...g.tabs.children];
    children.splice(g.index + 1, 0, pane);
    return replaceAt(root, g.path, { type: 'tabs', active: g.index + 1, children });
  }
  const p = pathOf(root, target);
  if (!p) return root;
  return replaceAt(root, p, { type: 'tabs', active: 1, children: [at(root, p) as Pane, pane] });
}

function activateIn(root: LayoutNode, id: string): LayoutNode {
  const g = groupOf(root, id);
  return g && g.tabs.active !== g.index ? replaceAt(root, g.path, { ...g.tabs, active: g.index }) : root;
}

// Keeps focus and zoom on panes that exist; the first pane takes focus otherwise.
function fix(ws: Workspace): Workspace {
  const panes = panesOf(ws.layout.root);
  const focus = ws.focus && panes.some((p) => p.id === ws.focus) ? ws.focus : panes[0]?.id;
  const zoom = ws.zoom && panes.some((p) => p.id === ws.zoom) && panes.length > 1 ? ws.zoom : undefined;
  return { layout: ws.layout, ...(focus ? { focus } : {}), ...(zoom ? { zoom } : {}) };
}

const agentPane = (id: string, agent: string): Pane => ({ type: 'pane', id, panel: 'agent', bind: { agent } });

// A grid of `cols` × `rows` filled row by row; agents beyond the last cell become tabs in it. Ids are reused for agents that
// already had a pane, so their terminals stay as they are.
function buildPreset(preset: Preset, agents: readonly string[], old: LayoutNode): LayoutNode {
  const reuse = new Map<string, string>();
  for (const p of panesOf(old)) { const a = bindOf(p); if (a && !reuse.has(a)) reuse.set(a, p.id); }
  const taken = new Set(reuse.values());
  let n = 0;
  const id = (agent?: string) => {
    const r = agent ? reuse.get(agent) : undefined;
    if (r) { reuse.delete(agent as string); return r; }
    let next: string;
    do { next = `p${++n}`; } while (taken.has(next));
    taken.add(next);
    return next;
  };
  const list = [...new Set(agents)].slice(0, MAX_PANES);
  const cell = (from: readonly string[]): Pane | Tabs => {
    if (from.length === 0) return emptyPane(id());
    if (from.length === 1) return agentPane(id(from[0]), from[0] as string);
    return { type: 'tabs', active: 0, children: from.map((a) => agentPane(id(a), a)) };
  };
  if (preset === '1x1') {
    const c = cell(list);
    return c.type === 'pane' && c.panel !== 'empty' ? { type: 'tabs', active: 0, children: [c] } : c;
  }
  if (preset === 'focus') {
    const strip = [1, 2, 3].map((i) => cell(i < 3 ? list.slice(i, i + 1) : list.slice(3)));
    return { type: 'split', direction: 'row', ratios: [0.7, 0.3], children: [cell(list.slice(0, 1)), { type: 'split', direction: 'column', ratios: [1 / 3, 1 / 3, 1 / 3], children: strip }] };
  }
  const [cols, rows] = preset.split('x').map(Number) as [number, number];
  const cells = cols * rows;
  const fill = Array.from({ length: cells }, (_, i) => cell(i < cells - 1 ? list.slice(i, i + 1) : list.slice(i)));
  const row = (r: number): LayoutNode => cols === 1 ? fill[r] as LayoutNode : { type: 'split', direction: 'row', ratios: Array(cols).fill(1 / cols), children: fill.slice(r * cols, r * cols + cols) };
  if (rows === 1) return row(0);
  return { type: 'split', direction: 'column', ratios: Array(rows).fill(1 / rows), children: Array.from({ length: rows }, (_, r) => row(r)) };
}

function step(ws: Workspace, action: Action): Workspace {
  const root = ws.layout.root;
  const has = (id: string) => !!pathOf(root, id);
  const set = (next: LayoutNode | undefined, extra: Partial<Workspace> = {}): Workspace => {
    const r = next ? normalise(next) : undefined;
    return { ...ws, ...extra, layout: { version: LAYOUT_VERSION, root: r ?? emptyPane(freshId(root)) } };
  };
  switch (action.type) {
    case 'split': {
      if (!has(action.pane)) return ws;
      const id = freshId(root);
      const pane: Pane = { type: 'pane', id, panel: action.panel ?? 'empty', ...(action.bind ? { bind: action.bind } : {}) };
      return set(insertBeside(root, action.pane, pane, action.direction, true), { focus: id, zoom: undefined });
    }
    case 'close': {
      if (!has(action.pane)) return ws;
      // The pane next to it in its tab group takes focus, else whatever comes first.
      const g = groupOf(root, action.pane);
      const neighbour = g ? (g.tabs.children[g.index + 1] ?? g.tabs.children[g.index - 1])?.id : undefined;
      return set(remove(root, action.pane), { focus: ws.focus === action.pane ? neighbour : ws.focus });
    }
    case 'resize': {
      const s = at(root, action.path);
      const i = action.index;
      if (s?.type !== 'split' || i < 0 || i + 1 >= s.children.length || !Number.isFinite(action.delta)) return ws;
      const a = s.ratios[i] as number;
      const b = s.ratios[i + 1] as number;
      const min = Math.min(MIN_RATIO, (a + b) / 2);
      const nextA = Math.min(a + b - min, Math.max(min, a + action.delta));
      if (nextA === a) return ws;
      const ratios = [...s.ratios];
      ratios[i] = nextA;
      ratios[i + 1] = a + b - nextA;
      return { ...ws, layout: { version: LAYOUT_VERSION, root: replaceAt(root, action.path, { ...s, ratios }) } };
    }
    case 'swap': {
      const pa = pathOf(root, action.a);
      const pb = pathOf(root, action.b);
      if (!pa || !pb || action.a === action.b) return ws;
      const a = at(root, pa) as Pane;
      const b = at(root, pb) as Pane;
      return set(replaceAt(replaceAt(root, pa, b), pb, a));
    }
    case 'move': {
      if (action.pane === action.target || !has(action.pane) || !has(action.target)) return ws;
      const pane = at(root, pathOf(root, action.pane) as Path) as Pane;
      const g = groupOf(root, action.pane);
      // A tab dropped on a tab of its own group takes that tab's place (reordering the tabs).
      if (action.place === 'center' && g && g.tabs.children.some((c) => c.id === action.target)) {
        const to = g.tabs.children.findIndex((c) => c.id === action.target);
        const children = g.tabs.children.filter((c) => c.id !== action.pane);
        children.splice(to, 0, pane);
        return set(replaceAt(root, g.path, { type: 'tabs', active: to, children }), { focus: action.pane, zoom: undefined });
      }
      const without = remove(root, action.pane);
      if (!without || !pathOf(without, action.target)) return ws;
      const next = action.place === 'center'
        ? addTab(without, action.target, pane)
        : insertBeside(without, action.target, pane, action.place === 'start' || action.place === 'end' ? 'row' : 'column', action.place === 'end' || action.place === 'bottom');
      return set(next, { focus: action.pane, zoom: undefined });
    }
    case 'detach': {
      const g = groupOf(root, action.pane);
      if (!g || g.tabs.children.length < 2) return ws;
      const pane = g.tabs.children[g.index] as Pane;
      const without = remove(root, action.pane) as LayoutNode;
      const stay = (g.tabs.children[g.index === 0 ? 1 : 0] as Pane).id;
      return set(insertBeside(without, stay, pane, 'row', true), { focus: action.pane, zoom: undefined });
    }
    case 'activate':
      if (!has(action.pane)) return ws;
      return { ...ws, focus: action.pane, layout: { ...ws.layout, root: activateIn(root, action.pane) } };
    case 'focus':
      return has(action.pane) && ws.focus !== action.pane ? { ...ws, focus: action.pane } : ws;
    case 'maximize':
      if (!has(action.pane)) return ws;
      return { ...ws, zoom: action.pane, focus: action.pane, layout: { ...ws.layout, root: activateIn(root, action.pane) } };
    case 'restore':
      return ws.zoom ? { layout: ws.layout, ...(ws.focus ? { focus: ws.focus } : {}) } : ws;
    case 'preset': {
      const next = buildPreset(action.preset, action.agents, root);
      return set(next, { focus: panesOf(next)[0]?.id, zoom: undefined });
    }
    case 'bind': {
      const p = pathOf(root, action.pane);
      if (!p) return ws;
      const pane = at(root, p) as Pane;
      const { bind: _old, ...rest } = pane;
      const next: Pane = { ...rest, panel: action.panel ?? (pane.panel === 'empty' && action.bind ? 'agent' : pane.panel), ...(action.bind ? { bind: action.bind } : {}) };
      return set(replaceAt(root, p, next), { focus: action.pane });
    }
    case 'open': {
      // Already shown somewhere: bring it to the front there.
      const shown = panesOf(root).find((p) => bindOf(p) === action.agent);
      if (shown) return action.background ? ws : step(ws, { type: 'activate', pane: shown.id });
      const panes = panesOf(root);
      const target = panes.find((p) => p.id === ws.focus) ?? panes[0];
      if (!target) return ws;
      const id = freshId(root);
      const pane = agentPane(id, action.agent);
      // The window's only pane is empty: the agent becomes its first tab.
      if (panes.length === 1 && target.panel === 'empty') return set({ type: 'tabs', active: 0, children: [pane] }, { focus: id, zoom: undefined });
      if (target.panel === 'empty' && !action.background) return step(ws, { type: 'bind', pane: target.id, bind: { agent: action.agent }, panel: 'agent' });
      const added = addTab(root, target.id, pane);
      if (!action.background) return set(added, { focus: id, zoom: undefined });
      // In the background the tab is added without taking the front, unless the group showed nothing.
      const g = groupOf(root, target.id);
      const keep = g ? g.tabs.children[g.tabs.active] : target;
      return set(keep && keep.panel !== 'empty' ? activateIn(added, keep.id) : added);
    }
    case 'show': {
      // A panel that is not about one agent (the theme editor): brought to the front where it is, else put beside the pane in
      // focus so what it changes stays in view (as a tab there when the limits leave no room for another pane).
      const shown = panesOf(root).find((p) => p.panel === action.panel);
      if (shown) return { ...step(ws, { type: 'activate', pane: shown.id }), ...(ws.zoom && ws.zoom !== shown.id ? { zoom: undefined } : {}) };
      const panes = panesOf(root);
      const target = panes.find((p) => p.id === ws.focus) ?? panes[0];
      if (!target) return ws;
      if (target.panel === 'empty') return step(ws, { type: 'bind', pane: target.id, panel: action.panel });
      const fits = (n: LayoutNode) => panesOf(n).length <= MAX_PANES && depthOf(n) <= MAX_DEPTH;
      const split = step(ws, { type: 'split', pane: target.id, direction: 'row', panel: action.panel });
      if (fits(split.layout.root)) return split;
      // No room beside it: a tab next to the pane in focus, else next to any pane where a tab fits.
      const id = freshId(root);
      for (const p of [target, ...panes.filter((x) => x.id !== target.id)]) {
        const next = normalise(addTab(root, p.id, { type: 'pane', id, panel: action.panel }));
        if (next && fits(next)) return set(next, { focus: id, zoom: undefined });
      }
      return ws;
    }
    case 'prune': {
      // Agents that are gone: a tab closes; a pane of its own stays in its place, empty, so the arrangement does not shift.
      let next: LayoutNode | undefined = root;
      for (const p of panesOf(root)) {
        const a = bindOf(p);
        if (!a || action.alive.has(a) || !next) continue;
        if (groupOf(next, p.id) || panesOf(next).length === 1) next = remove(next, p.id);
        else { const path = pathOf(next, p.id) as Path; next = replaceAt(next, path, emptyPane(p.id)); }
      }
      return next === root ? ws : set(next);
    }
    case 'replace':
      return { layout: action.layout };
  }
}

export function reduce(ws: Workspace, action: Action): Workspace {
  const next = step(ws, action);
  if (next === ws) return ws;
  const panes = panesOf(next.layout.root);
  // The limits hold after every step: an action that would break them is refused, not repaired.
  if (panes.length > MAX_PANES || depthOf(next.layout.root) > MAX_DEPTH) return ws;
  return fix(next);
}

// The panes on screen: the maximized one alone, else every lone pane and the front tab of every group. On a narrow screen the
// tree is one tab group, so only the pane in focus shows.
export function visiblePanes(ws: Workspace, narrow = false): Pane[] {
  const panes = panesOf(ws.layout.root);
  if (ws.zoom) return panes.filter((p) => p.id === ws.zoom);
  if (narrow) { const f = panes.find((p) => p.id === ws.focus) ?? panes[0]; return f ? [f] : []; }
  const out: Pane[] = [];
  const walk = (n: LayoutNode) => {
    if (n.type === 'pane') out.push(n);
    else if (n.type === 'tabs') out.push(n.children[n.active] as Pane);
    else n.children.forEach(walk);
  };
  walk(ws.layout.root);
  return out;
}

// What is drawn: the tree, or (maximized) the one pane, or (narrow) every pane as tabs of one group with the focused one in front.
export function displayRoot(ws: Workspace, narrow: boolean): LayoutNode {
  const panes = panesOf(ws.layout.root);
  if (ws.zoom) return panes.find((p) => p.id === ws.zoom) ?? ws.layout.root;
  if (narrow && panes.length > 1) return { type: 'tabs', active: Math.max(0, panes.findIndex((p) => p.id === ws.focus)), children: panes };
  return ws.layout.root;
}
