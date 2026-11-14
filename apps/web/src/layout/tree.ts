// The layout tree (ADR-0068, docs/ux/layout.md): splits, tab groups and panes, as plain versioned data that every client can read
// and write. Nothing here touches the DOM; the window draws what this describes.

export const LAYOUT_VERSION = 1;
export const MAX_DEPTH = 6; // nodes on the longest path from the root to a pane
export const MAX_PANES = 32;
export const MIN_RATIO = 0.1; // the smallest share a split gives one child
const MAX_ID = 64;

// The panels this window knows. A layout may name others (a newer client's): they draw an "unknown panel" placeholder.
export const PANELS = ['terminal', 'conversation', 'agent', 'approvals', 'changes', 'files', 'tasks', 'messages', 'usage', 'theme', 'empty'] as const;
export type KnownPanel = (typeof PANELS)[number];
export const isKnownPanel = (id: string): id is KnownPanel => (PANELS as readonly string[]).includes(id);
// Panels that show one agent, chosen by the pane's binding.
export const AGENT_PANELS: readonly string[] = ['terminal', 'conversation', 'agent', 'files'];

export type Follow = 'attention' | 'selection';
export type Bind = { agent: string } | { follow: Follow };
export interface Pane { type: 'pane'; id: string; panel: string; bind?: Bind }
export interface Tabs { type: 'tabs'; active: number; children: Pane[] }
export interface Split { type: 'split'; direction: 'row' | 'column'; ratios: number[]; children: LayoutNode[] }
export type LayoutNode = Pane | Tabs | Split;
export interface Layout { version: typeof LAYOUT_VERSION; root: LayoutNode }

export type LayoutError = 'json' | 'invalid' | 'newer' | 'too-deep' | 'too-many';
export type Parsed = { ok: true; layout: Layout } | { ok: false; error: LayoutError };

export const emptyPane = (id = 'p1'): Pane => ({ type: 'pane', id, panel: 'empty' });
export const defaultLayout = (): Layout => ({ version: LAYOUT_VERSION, root: emptyPane() });

// Every pane, in reading order (row by row, tabs in order).
export function panesOf(node: LayoutNode): Pane[] {
  if (node.type === 'pane') return [node];
  if (node.type === 'tabs') return [...node.children];
  return node.children.flatMap(panesOf);
}

export function depthOf(node: LayoutNode): number {
  if (node.type === 'pane') return 1;
  if (node.type === 'tabs') return 2;
  return 1 + Math.max(...node.children.map(depthOf));
}

export const bindOf = (pane: Pane): string | undefined => (pane.bind && 'agent' in pane.bind ? pane.bind.agent : undefined);
export const followOf = (pane: Pane): Follow | undefined => (pane.bind && 'follow' in pane.bind ? pane.bind.follow : undefined);

// A pane id nobody in `node` uses: `p<n>`, one past the highest. Deterministic, so the reducer stays a pure function.
export function freshId(node: LayoutNode, taken: ReadonlySet<string> = new Set()): string {
  let n = 0;
  for (const p of panesOf(node)) { const m = /^p(\d+)$/.exec(p.id); if (m) n = Math.max(n, Number(m[1])); }
  for (const id of taken) { const m = /^p(\d+)$/.exec(id); if (m) n = Math.max(n, Number(m[1])); }
  return `p${n + 1}`;
}

class Invalid extends Error {}
const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

function readBind(v: unknown): Bind | undefined {
  if (!isObject(v)) return undefined;
  if (typeof v.agent === 'string' && v.agent.length > 0 && v.agent.length <= 256) return { agent: v.agent };
  if (v.follow === 'attention' || v.follow === 'selection') return { follow: v.follow };
  return undefined; // an unknown binding (a newer client's) shows the pane unbound rather than refusing the layout
}

function readPane(v: Record<string, unknown>): Pane {
  if (typeof v.panel !== 'string' || v.panel.length === 0 || v.panel.length > MAX_ID) throw new Invalid();
  const bind = readBind(v.bind);
  return { type: 'pane', id: typeof v.id === 'string' && v.id.length > 0 && v.id.length <= MAX_ID ? v.id : '', panel: v.panel, ...(bind ? { bind } : {}) };
}

function equalRatios(n: number): number[] { return Array.from({ length: n }, () => 1 / n); }

// Positive and finite, summing to 1; anything else is shared out equally.
export function normaliseRatios(ratios: readonly number[], n: number): number[] {
  if (ratios.length !== n || n === 0 || ratios.some((r) => typeof r !== 'number' || !Number.isFinite(r) || r <= 0)) return equalRatios(n);
  const sum = ratios.reduce((a, b) => a + b, 0);
  // Already normal: kept as is, so reading a saved layout back gives exactly what was saved.
  if (Math.abs(sum - 1) < 1e-9) return [...ratios];
  return ratios.map((r) => r / sum);
}

// Reads untrusted JSON into a node. `undefined` means the node held nothing (an empty split or tab group): it is dropped.
function readNode(v: unknown, depth: number): LayoutNode | undefined {
  if (!isObject(v)) throw new Invalid();
  if (depth > MAX_DEPTH * 4) throw new Invalid(); // guards the recursion; the real depth rule runs after normalising
  if (v.type === 'pane') return readPane(v);
  if (v.type === 'tabs') {
    if (!Array.isArray(v.children)) throw new Invalid();
    const children: Pane[] = [];
    for (const c of v.children) {
      const n = readNode(c, depth + 1);
      if (!n) continue;
      if (n.type === 'split') throw new Invalid(); // a tab holds one pane, never a split
      children.push(...panesOf(n));
    }
    if (children.length === 0) return undefined;
    const active = typeof v.active === 'number' && Number.isInteger(v.active) ? Math.min(children.length - 1, Math.max(0, v.active)) : 0;
    return { type: 'tabs', active, children };
  }
  if (v.type === 'split') {
    if (v.direction !== 'row' && v.direction !== 'column') throw new Invalid();
    if (!Array.isArray(v.children)) throw new Invalid();
    const raw = Array.isArray(v.ratios) ? v.ratios : [];
    const children: LayoutNode[] = [];
    const ratios: number[] = [];
    const usable = raw.length === v.children.length && raw.every((r) => typeof r === 'number' && Number.isFinite(r) && r > 0);
    v.children.forEach((c: unknown, i: number) => {
      const n = readNode(c, depth + 1);
      if (!n) return;
      children.push(n);
      ratios.push(usable ? (raw[i] as number) : 1);
    });
    return normalise({ type: 'split', direction: v.direction, ratios, children });
  }
  throw new Invalid();
}

// The tree's canonical form: no split with fewer than two children, no split inside a split of the same direction, ratios that
// are positive, at least MIN_RATIO-ish and sum to 1, tab groups with a valid active index. Every operation ends here.
export function normalise(node: LayoutNode): LayoutNode | undefined {
  if (node.type === 'pane') return node;
  if (node.type === 'tabs') {
    if (node.children.length === 0) return undefined;
    return { type: 'tabs', active: Math.min(node.children.length - 1, Math.max(0, Math.trunc(node.active) || 0)), children: node.children };
  }
  const base = normaliseRatios(node.ratios, node.children.length);
  const children: LayoutNode[] = [];
  const ratios: number[] = [];
  node.children.forEach((c, i) => {
    const n = normalise(c);
    if (!n) return;
    if (n.type === 'split' && n.direction === node.direction) {
      n.children.forEach((g, j) => { children.push(g); ratios.push((base[i] as number) * (n.ratios[j] as number)); });
    } else { children.push(n); ratios.push(base[i] as number); }
  });
  if (children.length === 0) return undefined;
  if (children.length === 1) return children[0];
  return { type: 'split', direction: node.direction, ratios: normaliseRatios(ratios, children.length), children };
}

// Missing or repeated pane ids get fresh ones, so every pane can be addressed and keyed.
function fixIds(node: LayoutNode): LayoutNode {
  const seen = new Set<string>();
  const all = panesOf(node).map((p) => p.id);
  const fix = (p: Pane): Pane => {
    if (p.id && !seen.has(p.id)) { seen.add(p.id); return p; }
    const id = freshId(node, new Set([...all, ...seen]));
    seen.add(id);
    return { ...p, id };
  };
  const walk = (n: LayoutNode): LayoutNode => n.type === 'pane' ? fix(n) : n.type === 'tabs' ? { ...n, children: n.children.map(fix) } : { ...n, children: n.children.map(walk) };
  return walk(node);
}

// The one door from untrusted data (storage, an imported file) into a layout. Never throws.
export function parseLayout(value: unknown): Parsed {
  if (!isObject(value)) return { ok: false, error: 'invalid' };
  if (typeof value.version !== 'number' || !Number.isInteger(value.version) || value.version < 1) return { ok: false, error: 'invalid' };
  if (value.version > LAYOUT_VERSION) return { ok: false, error: 'newer' };
  let root: LayoutNode | undefined;
  try { root = readNode(value.root, 1); } catch { return { ok: false, error: 'invalid' }; }
  root = root ?? emptyPane();
  if (panesOf(root).length > MAX_PANES) return { ok: false, error: 'too-many' };
  if (depthOf(root) > MAX_DEPTH) return { ok: false, error: 'too-deep' };
  return { ok: true, layout: { version: LAYOUT_VERSION, root: fixIds(root) } };
}

export function deserialiseLayout(text: string): Parsed {
  let value: unknown;
  try { value = JSON.parse(text); } catch { return { ok: false, error: 'json' }; }
  return parseLayout(value);
}

export function serialiseLayout(layout: Layout): string {
  return JSON.stringify(layout, null, 2);
}

// The checks the tests hold every operation to.
export function isValid(layout: Layout): boolean {
  const ok = (n: LayoutNode, parent?: Split): boolean => {
    if (n.type === 'pane') return typeof n.id === 'string' && n.id.length > 0 && typeof n.panel === 'string' && n.panel.length > 0;
    if (n.type === 'tabs') return n.children.length > 0 && n.active >= 0 && n.active < n.children.length && n.children.every((c) => c.type === 'pane' && ok(c));
    const sum = n.ratios.reduce((a, b) => a + b, 0);
    return n.children.length >= 2 && n.ratios.length === n.children.length && Math.abs(sum - 1) < 1e-6 && n.ratios.every((r) => r > 0)
      && (!parent || parent.direction !== n.direction) && n.children.every((c) => ok(c, n));
  };
  const ids = panesOf(layout.root).map((p) => p.id);
  return layout.version === LAYOUT_VERSION && ok(layout.root) && new Set(ids).size === ids.length && ids.length <= MAX_PANES && depthOf(layout.root) <= MAX_DEPTH;
}
