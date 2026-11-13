import { beforeEach, describe, expect, it } from 'vitest';
import { reduce } from './engine';
import { CURRENT_KEY, SAVED_KEY, deleteNamed, forgetLoaded, loadWorkspace, migrate, renameNamed, saveNamed, saveWorkspace, savedLayouts } from './store';
import { bindOf, defaultLayout, panesOf } from './tree';

function memory(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, m };
}
const broken = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('full'); } };
const shownAgents = (s: { ws: { layout: { root: Parameters<typeof panesOf>[0] } } }) => panesOf(s.ws.layout.root).map((p) => bindOf(p) ?? p.panel);

beforeEach(() => forgetLoaded());

describe('the saved workspace', () => {
  it('starts as one empty pane when nothing is stored, and when storage throws', () => {
    expect(shownAgents(loadWorkspace(memory()))).toEqual(['empty']);
    forgetLoaded();
    expect(shownAgents(loadWorkspace(broken))).toEqual(['empty']);
    expect(() => saveWorkspace(loadWorkspace(broken), broken)).not.toThrow();
  });
  it('survives a reload: layout, focus, maximized pane and whether the user arranged it', () => {
    const kv = memory();
    let ws = reduce(loadWorkspace(kv).ws, { type: 'preset', preset: '2x2', agents: ['a', 'b', 'c', 'd'] });
    const c = panesOf(ws.layout.root)[2]!.id;
    ws = reduce(ws, { type: 'maximize', pane: c });
    saveWorkspace({ ws, touched: true }, kv);
    forgetLoaded();
    const back = loadWorkspace(kv);
    expect(back.ws).toEqual(ws);
    expect(back.touched).toBe(true);
  });
  it('ignores a stored layout it cannot read or that a newer version wrote', () => {
    for (const bad of ['{', JSON.stringify({ version: 1, layout: { version: 2, root: { type: 'pane', panel: 'x' } } }), JSON.stringify({ layout: 3 })]) {
      forgetLoaded();
      expect(shownAgents(loadWorkspace(memory({ [CURRENT_KEY]: bad })))).toEqual(['empty']);
    }
  });
  it('a focus or zoom naming a pane that is not there is dropped', () => {
    const kv = memory({ [CURRENT_KEY]: JSON.stringify({ version: 1, layout: defaultLayout(), focus: 'zz', zoom: 'zz' }) });
    const s = loadWorkspace(kv);
    expect(s.ws.focus).toBe('p1');
    expect(s.ws.zoom).toBeUndefined();
  });
});

describe('moving the old tabs into the layout', () => {
  it('keeps every open tab and the one in front', () => {
    const kv = memory({ 'agora.layout': JSON.stringify({ pinned: ['a', 'b', 'c'], active: 'b', layout: 'single', touched: true, showAgents: true }) });
    const s = loadWorkspace(kv);
    expect(s.ws.layout.root).toMatchObject({ type: 'tabs', active: 1 });
    expect(shownAgents(s)).toEqual(['a', 'b', 'c']);
    expect(s.touched).toBe(true);
  });
  it('side by side becomes a 2 × 2 grid of the first four, the rest as tabs', () => {
    const s = migrate({ pinned: ['a', 'b', 'c', 'd', 'e'], layout: 'grid' });
    expect(s && shownAgents(s)).toEqual(['a', 'b', 'c', 'd', 'e']);
    expect(s?.ws.layout.root.type).toBe('split');
  });
  it('drops junk in the old record', () => {
    expect(migrate({ pinned: [1, '', 'a', 'a', null] }) && shownAgents(migrate({ pinned: [1, '', 'a', 'a', null] })!)).toEqual(['a']);
    expect(migrate({ pinned: 'a' })).toBeUndefined();
    expect(migrate(null)).toBeUndefined();
    expect(migrate({ pinned: [] }) && shownAgents(migrate({ pinned: [] })!)).toEqual(['empty']);
  });
  it('the new record wins over the old one', () => {
    const ws = reduce(loadWorkspace(memory()).ws, { type: 'preset', preset: '2x1', agents: ['x', 'y'] });
    forgetLoaded();
    const kv = memory({ 'agora.layout': JSON.stringify({ pinned: ['a'] }), [CURRENT_KEY]: JSON.stringify({ version: 1, layout: ws.layout }) });
    expect(shownAgents(loadWorkspace(kv))).toEqual(['x', 'y']);
  });
});

describe('named layouts', () => {
  it('saves, replaces by name, renames and deletes', () => {
    const kv = memory();
    const one = reduce(loadWorkspace(kv).ws, { type: 'preset', preset: '2x2', agents: [] }).layout;
    expect(saveNamed('  Review  ', one, kv).map((x) => x.name)).toEqual(['Review']);
    expect(saveNamed('Watch', defaultLayout(), kv).map((x) => x.name)).toEqual(['Watch', 'Review']);
    expect(saveNamed('Review', defaultLayout(), kv).map((x) => x.name)).toEqual(['Review', 'Watch']);
    expect(savedLayouts(kv)[0]!.layout).toEqual(defaultLayout());
    expect(renameNamed('Watch', 'Review', kv).map((x) => x.name)).toEqual(['Review', 'Watch']); // taken: refused
    expect(renameNamed('Watch', 'Night', kv).map((x) => x.name)).toEqual(['Review', 'Night']);
    expect(deleteNamed('Review', kv).map((x) => x.name)).toEqual(['Night']);
    expect(saveNamed('   ', one, kv).map((x) => x.name)).toEqual(['Night']);
  });
  it('skips saved layouts it cannot read, keeps the rest', () => {
    const kv = memory({ [SAVED_KEY]: JSON.stringify({ version: 1, items: [{ name: 'new', layout: { version: 9, root: {} } }, { name: 'ok', layout: defaultLayout() }, { layout: defaultLayout() }, 'x'] }) });
    expect(savedLayouts(kv).map((x) => x.name)).toEqual(['ok']);
    expect(savedLayouts(memory({ [SAVED_KEY]: '{' }))).toEqual([]);
    expect(savedLayouts(broken)).toEqual([]);
  });
});
