import { describe, expect, it } from 'vitest';
import { type Action, type Place, type Workspace, PRESETS, displayRoot, findPane, pathOf, reduce, visiblePanes, workspaceOf } from './engine';
import { type Layout, type LayoutNode, MAX_PANES, MIN_RATIO, bindOf, defaultLayout, deserialiseLayout, isValid, panesOf, parseLayout, serialiseLayout } from './tree';

const ws0 = (): Workspace => workspaceOf(defaultLayout());
const run = (ws: Workspace, ...actions: Action[]) => actions.reduce(reduce, ws);
const agents = (ws: Workspace) => panesOf(ws.layout.root).map((p) => bindOf(p) ?? p.panel);
const shape = (n: LayoutNode): unknown => n.type === 'pane' ? (bindOf(n) ?? n.panel) : n.type === 'tabs' ? { tabs: n.children.map(shape), active: n.active } : { [n.direction]: n.children.map(shape) };
const grid = (preset: (typeof PRESETS)[number], list: string[]) => run(ws0(), { type: 'preset', preset, agents: list });

describe('presets', () => {
  it('1x1 is one tab group of every agent, like the window before layouts', () => {
    const ws = grid('1x1', ['a', 'b', 'c']);
    expect(shape(ws.layout.root)).toEqual({ tabs: ['a', 'b', 'c'], active: 0 });
    expect(visiblePanes(ws).map(bindOf)).toEqual(['a']);
  });
  it('2x1 puts two side by side, 1x2 stacks them', () => {
    expect(shape(grid('2x1', ['a', 'b']).layout.root)).toEqual({ row: ['a', 'b'] });
    expect(shape(grid('1x2', ['a', 'b']).layout.root)).toEqual({ column: ['a', 'b'] });
  });
  it('2x2 fills row by row', () => {
    expect(shape(grid('2x2', ['a', 'b', 'c', 'd']).layout.root)).toEqual({ column: [{ row: ['a', 'b'] }, { row: ['c', 'd'] }] });
  });
  it('3x2 is three columns, two rows', () => {
    const ws = grid('3x2', ['a', 'b', 'c', 'd', 'e', 'f']);
    expect(shape(ws.layout.root)).toEqual({ column: [{ row: ['a', 'b', 'c'] }, { row: ['d', 'e', 'f'] }] });
    expect(visiblePanes(ws)).toHaveLength(6);
  });
  it('agents beyond the cells become tabs in the last pane', () => {
    expect(shape(grid('2x1', ['a', 'b', 'c']).layout.root)).toEqual({ row: ['a', { tabs: ['b', 'c'], active: 0 }] });
  });
  it('cells without an agent are empty panes', () => {
    expect(shape(grid('2x2', ['a']).layout.root)).toEqual({ column: [{ row: ['a', 'empty'] }, { row: ['empty', 'empty'] }] });
  });
  it('focus + strip: one large pane and a column of three', () => {
    const ws = grid('focus', ['a', 'b', 'c', 'd', 'e']);
    expect(shape(ws.layout.root)).toEqual({ row: ['a', { column: ['b', 'c', { tabs: ['d', 'e'], active: 0 }] }] });
    expect(ws.layout.root.type === 'split' && ws.layout.root.ratios[0]).toBeCloseTo(0.7);
  });
  it('keeps the pane ids of agents already shown, so their terminals stay', () => {
    const before = grid('2x2', ['a', 'b', 'c', 'd']);
    const id = (ws: Workspace, a: string) => panesOf(ws.layout.root).find((p) => bindOf(p) === a)?.id;
    const after = reduce(before, { type: 'preset', preset: '3x2', agents: ['d', 'c', 'b', 'a', 'e'] });
    for (const a of ['a', 'b', 'c', 'd']) expect(id(after, a)).toBe(id(before, a));
    expect(isValid(after.layout)).toBe(true);
  });
  it('never makes more panes than the limit', () => {
    const ws = grid('3x2', Array.from({ length: 100 }, (_, i) => `a${i}`));
    expect(panesOf(ws.layout.root)).toHaveLength(MAX_PANES);
    expect(isValid(ws.layout)).toBe(true);
  });
});

describe('operations', () => {
  it('split right and down add an empty pane next to the one asked, and focus it', () => {
    let ws = grid('1x1', ['a']);
    const a = (findPane(ws.layout, ws.focus) as { id: string }).id;
    ws = reduce(ws, { type: 'split', pane: a, direction: 'row' });
    expect(shape(ws.layout.root)).toEqual({ row: [{ tabs: ['a'], active: 0 }, 'empty'] });
    const b = ws.focus as string;
    ws = reduce(ws, { type: 'split', pane: b, direction: 'column', bind: { agent: 'c' } });
    expect(shape(ws.layout.root)).toEqual({ row: [{ tabs: ['a'], active: 0 }, { column: ['empty', 'c'] }] });
  });
  it('splitting inside a split of the same direction shares the pane\'s space', () => {
    let ws = grid('2x1', ['a', 'b']);
    const [pa] = panesOf(ws.layout.root);
    ws = reduce(ws, { type: 'split', pane: pa!.id, direction: 'row' });
    const root = ws.layout.root;
    expect(root.type === 'split' && root.ratios).toEqual([0.25, 0.25, 0.5]);
  });
  it('closing a pane gives its space to its siblings', () => {
    let ws = grid('3x2', ['a', 'b', 'c', 'd', 'e', 'f']);
    const b = panesOf(ws.layout.root)[1]!.id;
    ws = reduce(ws, { type: 'close', pane: b });
    expect(shape(ws.layout.root)).toEqual({ column: [{ row: ['a', 'c'] }, { row: ['d', 'e', 'f'] }] });
    const row = ws.layout.root.type === 'split' ? ws.layout.root.children[0] : undefined;
    expect(row?.type === 'split' && row.ratios).toEqual([0.5, 0.5]);
  });
  it('closing the last pane of a split collapses it', () => {
    let ws = grid('2x1', ['a', 'b']);
    ws = reduce(ws, { type: 'close', pane: panesOf(ws.layout.root)[0]!.id });
    expect(shape(ws.layout.root)).toEqual('b');
    ws = reduce(ws, { type: 'close', pane: panesOf(ws.layout.root)[0]!.id });
    expect(shape(ws.layout.root)).toEqual('empty');
  });
  it('closing a tab activates the next one', () => {
    let ws = grid('1x1', ['a', 'b', 'c']);
    ws = reduce(ws, { type: 'activate', pane: panesOf(ws.layout.root)[1]!.id });
    ws = reduce(ws, { type: 'close', pane: ws.focus as string });
    expect(shape(ws.layout.root)).toEqual({ tabs: ['a', 'c'], active: 1 });
    expect(bindOf(findPane(ws.layout, ws.focus)!)).toBe('c');
  });
  it('resizes two siblings and keeps a minimum share', () => {
    let ws = grid('2x1', ['a', 'b']);
    ws = reduce(ws, { type: 'resize', path: [], index: 0, delta: 0.2 });
    expect(ws.layout.root.type === 'split' && ws.layout.root.ratios[0]).toBeCloseTo(0.7);
    ws = reduce(ws, { type: 'resize', path: [], index: 0, delta: 5 });
    expect(ws.layout.root.type === 'split' && ws.layout.root.ratios[1]).toBeCloseTo(MIN_RATIO);
    const same = reduce(ws, { type: 'resize', path: [], index: 0, delta: 1 });
    expect(same).toBe(ws);
    expect(reduce(ws, { type: 'resize', path: [], index: 3, delta: 0.1 })).toBe(ws);
    expect(reduce(ws, { type: 'resize', path: [0], index: 0, delta: 0.1 })).toBe(ws);
  });
  it('resizing keeps every pane id (nothing is re-created)', () => {
    const ws = grid('2x2', ['a', 'b', 'c', 'd']);
    const next = reduce(ws, { type: 'resize', path: [0], index: 0, delta: -0.1 });
    expect(panesOf(next.layout.root).map((p) => p.id)).toEqual(panesOf(ws.layout.root).map((p) => p.id));
  });
  it('swaps two panes, ids travel with them', () => {
    let ws = grid('2x2', ['a', 'b', 'c', 'd']);
    const [a, , , d] = panesOf(ws.layout.root);
    ws = reduce(ws, { type: 'swap', a: a!.id, b: d!.id });
    expect(shape(ws.layout.root)).toEqual({ column: [{ row: ['d', 'b'] }, { row: ['c', 'a'] }] });
    expect(panesOf(ws.layout.root)[0]!.id).toBe(d!.id);
  });
  it('moves a pane into another pane\'s tabs and out again', () => {
    let ws = grid('2x1', ['a', 'b']);
    const [a, b] = panesOf(ws.layout.root);
    ws = reduce(ws, { type: 'move', pane: a!.id, target: b!.id, place: 'center' });
    expect(shape(ws.layout.root)).toEqual({ tabs: ['b', 'a'], active: 1 });
    ws = reduce(ws, { type: 'detach', pane: a!.id });
    expect(shape(ws.layout.root)).toEqual({ row: [{ tabs: ['b'], active: 0 }, 'a'] });
  });
  it('moves a pane to an edge of another', () => {
    let ws = grid('2x1', ['a', 'b']);
    const [a, b] = panesOf(ws.layout.root);
    const places: [Place, unknown][] = [['bottom', { column: ['b', 'a'] }], ['top', { column: ['a', 'b'] }], ['start', { row: ['a', 'b'] }], ['end', { row: ['b', 'a'] }]];
    for (const [place, want] of places) {
      ws = reduce(ws, { type: 'move', pane: a!.id, target: b!.id, place });
      expect(shape(ws.layout.root), place).toEqual(want);
    }
  });
  it('a move onto itself, or onto a pane that is not there, changes nothing', () => {
    const ws = grid('1x1', ['a', 'b']);
    const [a] = panesOf(ws.layout.root);
    expect(reduce(ws, { type: 'move', pane: a!.id, target: a!.id, place: 'end' })).toBe(ws);
    expect(reduce(ws, { type: 'move', pane: a!.id, target: 'nope', place: 'end' })).toBe(ws);
  });
  it('a tab dropped on a tab of its own group takes its place', () => {
    const ws = grid('1x1', ['a', 'b', 'c']);
    const [a, , c] = panesOf(ws.layout.root);
    expect(shape(reduce(ws, { type: 'move', pane: a!.id, target: c!.id, place: 'center' }).layout.root)).toEqual({ tabs: ['b', 'c', 'a'], active: 2 });
    expect(shape(reduce(ws, { type: 'move', pane: c!.id, target: a!.id, place: 'center' }).layout.root)).toEqual({ tabs: ['c', 'a', 'b'], active: 0 });
  });
  it('a tab moved to the edge of its own group leaves the group', () => {
    let ws = grid('1x1', ['a', 'b']);
    const [a, b] = panesOf(ws.layout.root);
    ws = reduce(ws, { type: 'move', pane: b!.id, target: a!.id, place: 'end' });
    expect(shape(ws.layout.root)).toEqual({ row: [{ tabs: ['a'], active: 0 }, 'b'] });
  });
  it('maximize shows one pane; restore brings the grid back as it was', () => {
    const ws = grid('2x2', ['a', 'b', 'c', 'd']);
    const c = panesOf(ws.layout.root)[2]!.id;
    const max = reduce(ws, { type: 'maximize', pane: c });
    expect(visiblePanes(max).map(bindOf)).toEqual(['c']);
    expect(displayRoot(max, false)).toEqual(findPane(max.layout, c));
    const back = reduce(max, { type: 'restore' });
    expect(back.layout).toEqual(ws.layout);
    expect(visiblePanes(back)).toHaveLength(4);
  });
  it('maximizing a background tab brings it to the front', () => {
    const ws = grid('1x1', ['a', 'b']);
    const b = panesOf(ws.layout.root)[1]!.id;
    expect(shape(reduce(ws, { type: 'maximize', pane: b }).layout.root)).toEqual({ tabs: ['a', 'b'], active: 1 });
  });
  it('binds a pane to an agent, to attention or to the selection', () => {
    let ws = grid('2x1', ['a']);
    const empty = panesOf(ws.layout.root)[1]!;
    ws = reduce(ws, { type: 'bind', pane: empty.id, bind: { follow: 'attention' } });
    expect(findPane(ws.layout, empty.id)).toEqual({ type: 'pane', id: empty.id, panel: 'agent', bind: { follow: 'attention' } });
    ws = reduce(ws, { type: 'bind', pane: empty.id, bind: { agent: 'z' } });
    expect(findPane(ws.layout, empty.id)?.bind).toEqual({ agent: 'z' });
    ws = reduce(ws, { type: 'bind', pane: empty.id, panel: 'approvals' });
    expect(findPane(ws.layout, empty.id)).toEqual({ type: 'pane', id: empty.id, panel: 'approvals' });
  });
  it('opening an agent: an empty window gets a tab, a shown one comes to the front, else a new tab in the focused pane', () => {
    let ws = ws0();
    ws = reduce(ws, { type: 'open', agent: 'a' });
    expect(shape(ws.layout.root)).toEqual({ tabs: ['a'], active: 0 });
    ws = reduce(ws, { type: 'open', agent: 'b' });
    expect(shape(ws.layout.root)).toEqual({ tabs: ['a', 'b'], active: 1 });
    ws = reduce(ws, { type: 'open', agent: 'a' });
    expect(shape(ws.layout.root)).toEqual({ tabs: ['a', 'b'], active: 0 });
    ws = reduce(ws, { type: 'open', agent: 'c', background: true });
    expect(shape(ws.layout.root)).toEqual({ tabs: ['a', 'c', 'b'], active: 0 });
  });
  it('opening into an empty pane of a grid fills that pane', () => {
    let ws = grid('2x1', ['a']);
    ws = reduce(ws, { type: 'focus', pane: panesOf(ws.layout.root)[1]!.id });
    ws = reduce(ws, { type: 'open', agent: 'b' });
    expect(shape(ws.layout.root)).toEqual({ row: ['a', 'b'] });
  });
  it('agents that are gone: their tabs close, a pane of their own stays empty', () => {
    let ws = grid('2x1', ['a', 'b', 'c']);
    ws = reduce(ws, { type: 'prune', alive: new Set(['b']) });
    expect(shape(ws.layout.root)).toEqual({ row: ['empty', { tabs: ['b'], active: 0 }] });
    expect(reduce(ws, { type: 'prune', alive: new Set(['b']) })).toBe(ws);
  });
  it('shows a panel (the theme editor): in an empty window, beside the pane in focus, or where it already is', () => {
    let ws = reduce(ws0(), { type: 'show', panel: 'theme' });
    expect(shape(ws.layout.root)).toBe('theme');
    ws = run(grid('2x1', ['a', 'b']), { type: 'focus', pane: panesOf(grid('2x1', ['a', 'b']).layout.root)[0]!.id }, { type: 'show', panel: 'theme' });
    expect(shape(ws.layout.root)).toEqual({ row: ['a', 'theme', 'b'] });
    expect(findPane(ws.layout, ws.focus)?.panel).toBe('theme');
    // Shown already, behind a tab or outside the maximized pane: brought to the front, nothing added.
    ws = run(grid('1x1', ['a', 'b']), { type: 'show', panel: 'theme' });
    const theme = panesOf(ws.layout.root).find((p) => p.panel === 'theme')!.id;
    ws = run(ws, { type: 'maximize', pane: panesOf(ws.layout.root)[0]!.id }, { type: 'show', panel: 'theme' });
    expect(ws.zoom).toBeUndefined();
    expect(visiblePanes(ws).map((p) => p.id)).toContain(theme);
    expect(panesOf(ws.layout.root).filter((p) => p.panel === 'theme')).toHaveLength(1);
    expect(isValid(ws.layout)).toBe(true);
  });
  it('showing a panel near the limits never breaks them', () => {
    const full = grid('3x2', Array.from({ length: MAX_PANES }, (_, i) => `a${i}`));
    const ws = reduce(full, { type: 'show', panel: 'theme' });
    expect(panesOf(ws.layout.root)).toHaveLength(MAX_PANES);
    expect(panesOf(ws.layout.root).some((p) => p.panel === 'theme')).toBe(false); // already at the limit: refused
    // Nested as deep as allowed: the panel still finds a place (beside, or as a tab), and the tree stays valid.
    let deep = grid('1x1', ['a']);
    for (let i = 0; i < 20; i++) deep = reduce(deep, { type: 'split', pane: deep.focus as string, direction: i % 2 ? 'row' : 'column', panel: 'terminal' });
    const shown = reduce(deep, { type: 'show', panel: 'theme' });
    expect(panesOf(shown.layout.root).some((p) => p.panel === 'theme')).toBe(true);
    expect(isValid(shown.layout)).toBe(true);
  });
  it('refuses a split beyond the pane limit', () => {
    let ws = grid('3x2', Array.from({ length: MAX_PANES }, (_, i) => `a${i}`));
    const before = ws;
    ws = reduce(ws, { type: 'split', pane: ws.focus as string, direction: 'row' });
    expect(ws).toBe(before);
  });
  it('refuses a split that would nest too deep', () => {
    let ws = ws0();
    for (let i = 0; i < 20; i++) ws = reduce(ws, { type: 'split', pane: ws.focus as string, direction: i % 2 ? 'row' : 'column' });
    expect(isValid(ws.layout)).toBe(true);
  });
  it('narrow screens draw every pane as tabs of one group, focus in front', () => {
    let ws = grid('2x2', ['a', 'b', 'c', 'd']);
    ws = reduce(ws, { type: 'focus', pane: panesOf(ws.layout.root)[2]!.id });
    expect(shape(displayRoot(ws, true))).toEqual({ tabs: ['a', 'b', 'c', 'd'], active: 2 });
    expect(visiblePanes(ws, true).map(bindOf)).toEqual(['c']);
  });
});

// A small deterministic generator, so a failure prints a seed that reproduces it.
function rng(seed: number) {
  let s = seed >>> 0;
  return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 2 ** 32; };
}

describe('every operation keeps the tree valid', () => {
  const pick = <T,>(r: () => number, list: readonly T[]): T => list[Math.floor(r() * list.length)] as T;
  for (let seed = 1; seed <= 60; seed++) {
    it(`random sequence ${seed}`, () => {
      const r = rng(seed);
      let ws = ws0();
      for (let i = 0; i < 120; i++) {
        const ids = panesOf(ws.layout.root).map((p) => p.id);
        const id = () => pick(r, [...ids, 'missing']);
        const before = panesOf(ws.layout.root).length;
        const splits: number[][] = [];
        const walk = (n: LayoutNode, p: number[]) => { if (n.type === 'split') { splits.push(p); n.children.forEach((c, j) => walk(c, [...p, j])); } };
        walk(ws.layout.root, []);
        const action: Action = pick(r, [
          () => ({ type: 'split', pane: id(), direction: pick(r, ['row', 'column'] as const) }),
          () => ({ type: 'close', pane: id() }),
          () => ({ type: 'resize', path: splits.length ? pick(r, splits) : [], index: Math.floor(r() * 3), delta: r() - 0.5 }),
          () => ({ type: 'swap', a: id(), b: id() }),
          () => ({ type: 'move', pane: id(), target: id(), place: pick(r, ['center', 'start', 'end', 'top', 'bottom'] as const) }),
          () => ({ type: 'detach', pane: id() }),
          () => ({ type: 'activate', pane: id() }),
          () => ({ type: 'maximize', pane: id() }),
          () => ({ type: 'restore' }),
          () => ({ type: 'preset', preset: pick(r, PRESETS), agents: Array.from({ length: Math.floor(r() * 9) }, (_, k) => `a${k}`) }),
          () => ({ type: 'bind', pane: id(), bind: pick(r, [{ agent: `a${Math.floor(r() * 5)}` }, { follow: 'attention' }, { follow: 'selection' }, undefined] as const) }),
          () => ({ type: 'open', agent: `a${Math.floor(r() * 12)}`, background: r() < 0.3 }),
          () => ({ type: 'prune', alive: new Set([`a${Math.floor(r() * 5)}`, `a${Math.floor(r() * 5)}`]) }),
        ] as (() => Action)[])();
        const next = reduce(ws, action);
        expect(isValid(next.layout), `seed ${seed} step ${i}: ${JSON.stringify(action)}`).toBe(true);
        const after = panesOf(next.layout.root).length;
        // The pane count moves exactly as the action says.
        if (next !== ws) {
          if (action.type === 'split') expect(after).toBe(before + 1);
          if (action.type === 'close') expect(after).toBe(Math.max(1, before - 1));
          if (['resize', 'swap', 'move', 'detach', 'activate', 'maximize', 'restore', 'bind', 'focus'].includes(action.type)) expect(after).toBe(before);
        }
        // Focus and zoom always name a pane of the tree.
        expect(next.focus && pathOf(next.layout.root, next.focus)).toBeTruthy();
        if (next.zoom) expect(pathOf(next.layout.root, next.zoom)).toBeTruthy();
        // What is on screen is a subset of the tree, without repeats, never empty.
        const shown = visiblePanes(next).map((p) => p.id);
        expect(shown.length).toBeGreaterThan(0);
        expect(new Set(shown).size).toBe(shown.length);
        // A serialised layout reads back the same.
        const back = deserialiseLayout(serialiseLayout(next.layout));
        expect(back.ok && back.layout).toEqual(next.layout);
        ws = next;
      }
    });
  }
});

describe('serialising and validating', () => {
  const ok = (v: unknown): Layout => { const p = parseLayout(v); if (!p.ok) throw new Error(p.error); return p.layout; };
  it('round-trips the spec examples', () => {
    for (const preset of PRESETS) {
      const l = grid(preset, ['a', 'b', 'c', 'd', 'e', 'f', 'g']).layout;
      expect(deserialiseLayout(serialiseLayout(l))).toEqual({ ok: true, layout: l });
    }
  });
  it('refuses what is not JSON or not a layout', () => {
    expect(deserialiseLayout('{')).toEqual({ ok: false, error: 'json' });
    for (const v of [null, [], 3, {}, { version: 1 }, { version: 1, root: { type: 'box' } }, { version: 0, root: { type: 'pane', panel: 'x' } }, { version: '1', root: {} },
      { version: 1, root: { type: 'split', direction: 'diagonal', children: [] } }, { version: 1, root: { type: 'pane' } },
      { version: 1, root: { type: 'tabs', children: [{ type: 'split', direction: 'row', children: [{ type: 'pane', panel: 'a' }, { type: 'pane', panel: 'b' }] }] } }]) {
      expect(parseLayout(v), JSON.stringify(v)).toEqual({ ok: false, error: 'invalid' });
    }
  });
  it('refuses a layout from a newer version', () => {
    expect(parseLayout({ version: 2, root: { type: 'pane', panel: 'terminal' } })).toEqual({ ok: false, error: 'newer' });
  });
  it('keeps unknown panels and drops unknown bindings', () => {
    const l = ok({ version: 1, root: { type: 'pane', id: 'x', panel: 'machines-map', bind: { project: 'p' } } });
    expect(l.root).toEqual({ type: 'pane', id: 'x', panel: 'machines-map' });
  });
  it('normalises ratios, collapses single-child splits and flattens same-direction splits', () => {
    const l = ok({ version: 1, root: { type: 'split', direction: 'row', ratios: [2, 2], children: [
      { type: 'pane', id: 'a', panel: 'empty' },
      { type: 'split', direction: 'row', ratios: [1, 3], children: [{ type: 'pane', id: 'b', panel: 'empty' }, { type: 'split', direction: 'column', children: [{ type: 'pane', id: 'c', panel: 'empty' }] }] },
    ] } });
    expect(l.root).toEqual({ type: 'split', direction: 'row', ratios: [0.5, 0.125, 0.375], children: [{ type: 'pane', id: 'a', panel: 'empty' }, { type: 'pane', id: 'b', panel: 'empty' }, { type: 'pane', id: 'c', panel: 'empty' }] });
    const bad = ok({ version: 1, root: { type: 'split', direction: 'column', ratios: [1, -1, 'x'], children: [{ type: 'pane', panel: 'a' }, { type: 'pane', panel: 'b' }] } });
    expect(bad.root.type === 'split' && bad.root.ratios).toEqual([0.5, 0.5]);
  });
  it('gives missing and repeated ids fresh ones and clamps the active tab', () => {
    const l = ok({ version: 1, root: { type: 'tabs', active: 9, children: [{ type: 'pane', id: 'p1', panel: 'a' }, { type: 'pane', id: 'p1', panel: 'b' }, { type: 'pane', panel: 'c' }] } });
    expect(panesOf(l.root).map((p) => p.id)).toEqual(['p1', 'p2', 'p3']);
    expect(l.root.type === 'tabs' && l.root.active).toBe(2);
    expect(isValid(l)).toBe(true);
  });
  it('an empty tree is one empty pane', () => {
    expect(ok({ version: 1, root: { type: 'split', direction: 'row', children: [] } }).root).toEqual({ type: 'pane', id: 'p1', panel: 'empty' });
  });
  it('refuses too many panes and too deep a tree', () => {
    const many = { version: 1, root: { type: 'tabs', children: Array.from({ length: MAX_PANES + 1 }, () => ({ type: 'pane', panel: 'empty' })) } };
    expect(parseLayout(many)).toEqual({ ok: false, error: 'too-many' });
    let deep: unknown = { type: 'pane', panel: 'empty' };
    for (let i = 0; i < 8; i++) deep = { type: 'split', direction: i % 2 ? 'row' : 'column', children: [deep, { type: 'pane', panel: 'empty' }] };
    expect(parseLayout({ version: 1, root: deep })).toEqual({ ok: false, error: 'too-deep' });
    let absurd: unknown = { type: 'pane', panel: 'empty' };
    for (let i = 0; i < 5000; i++) absurd = { type: 'split', direction: 'row', children: [absurd] };
    expect(parseLayout({ version: 1, root: absurd }).ok).toBe(false);
  });
});
