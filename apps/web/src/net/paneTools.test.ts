import { describe, expect, it } from 'vitest';
import type { AgentTileModel } from '../model';
import type { FileItem } from './files';
import { filterEntries, folderChoices, folderKey, kindOf, nextSort, parentOf, soleTarget, sortEntries, toolTarget } from './paneTools';

const agent = (over: Partial<AgentTileModel>): AgentTileModel => ({ id: 'n1/a', name: 'claude · amber', accent: '#fff', machine: 'laptop', workspace: 'app', project: 'app', state: 'idle', preview: '', workspaceRef: { node: 'n1', id: 'w1' }, ...over });
const f = (name: string, size = 0, isDir = false, secret = false): FileItem => ({ name, size, isDir, ...(secret ? { secret } : {}) });

describe('what kind a file is', () => {
  it('goes by the extension, folders and private files first', () => {
    expect(kindOf(f('src', 0, true))).toBe('folder');
    expect(kindOf(f('main.rs'))).toBe('code');
    expect(kindOf(f('README.md'))).toBe('text');
    expect(kindOf(f('Makefile'))).toBe('text');
    expect(kindOf(f('logo.PNG'))).toBe('image');
    expect(kindOf(f('Cargo.toml'))).toBe('config');
    expect(kindOf(f('a.tar'))).toBe('archive');
    expect(kindOf(f('blob.xyz'))).toBe('file');
    expect(kindOf(f('noext'))).toBe('file');
    expect(kindOf(f('.env', 4, false, true))).toBe('private');
    expect(kindOf(f('dir.rs', 0, true))).toBe('folder'); // a folder named like a source file stays a folder
  });
});

describe('sorting the list', () => {
  const items = [f('b.txt', 5), f('a.txt', 50), f('zdir', 0, true), f('adir', 0, true), f('c.rs', 20), f('file10'), f('file2')];
  const names = (l: FileItem[]) => l.map((i) => i.name);
  it('keeps folders first, whatever the column and direction', () => {
    for (const key of ['name', 'size', 'kind'] as const) for (const descending of [false, true]) {
      expect(names(sortEntries(items, { key, descending })).slice(0, 2).sort()).toEqual(['adir', 'zdir']);
    }
  });
  it('sorts names naturally (2 before 10), size either way, and breaks ties by name', () => {
    expect(names(sortEntries(items, { key: 'name', descending: false })).slice(2)).toEqual(['a.txt', 'b.txt', 'c.rs', 'file2', 'file10']);
    expect(names(sortEntries(items, { key: 'size', descending: true })).slice(2, 5)).toEqual(['a.txt', 'c.rs', 'b.txt']);
    expect(names(sortEntries(items, { key: 'size', descending: false })).slice(2, 4)).toEqual(['file2', 'file10']); // both 0: by name
  });
  it('does not change the list it was given', () => {
    const before = names(items);
    sortEntries(items, { key: 'size', descending: true });
    expect(names(items)).toEqual(before);
  });
  it('a second click on a column turns it round; a new column starts ascending', () => {
    expect(nextSort({ key: 'name', descending: false }, 'name')).toEqual({ key: 'name', descending: true });
    expect(nextSort({ key: 'name', descending: true }, 'size')).toEqual({ key: 'size', descending: false });
  });
});

describe('filtering by name', () => {
  it('matches any part of the name, any case, and a blank filter keeps everything', () => {
    const l = [f('Main.rs'), f('lib.rs'), f('README.md')];
    expect(filterEntries(l, 'MAIN').map((i) => i.name)).toEqual(['Main.rs']);
    expect(filterEntries(l, '.rs')).toHaveLength(2);
    expect(filterEntries(l, '  ')).toHaveLength(3);
    expect(filterEntries(l, 'zzz')).toEqual([]);
  });
  it('knows a parent folder', () => {
    expect(parentOf('a/b/c.rs')).toBe('a/b');
    expect(parentOf('a')).toBe('');
    expect(parentOf('')).toBe('');
  });
});

describe('the folders a tool pane can show', () => {
  const fleet = [
    agent({ id: 'n1/a', workspaceRef: { node: 'n1', id: 'w1' }, closed: true, filesChanged: 1 }),
    agent({ id: 'n1/b', workspaceRef: { node: 'n1', id: 'w1' }, filesChanged: 1 }), // the same folder, still running
    agent({ id: 'n2/c', machine: 'vps', workspaceRef: { node: 'n2', id: 'w9' }, project: 'app', filesChanged: 7 }),
    agent({ id: 'n3/d', project: 'docs', workspace: 'docs', workspaceRef: { node: 'n3', id: 'w3' } }),
    agent({ id: 'n3/e', project: 'none', workspaceRef: undefined }), // no folder reported: not offered
  ];
  it('lists one entry per folder, a running agent standing for it before a closed one, grouped by project', () => {
    const g = folderChoices(fleet);
    expect(g.map((x) => x.project)).toEqual(['app', 'docs']);
    expect(g[0]?.folders.map((x) => x.agent.id).sort()).toEqual(['n1/b', 'n2/c']);
    expect(g.flatMap((x) => x.folders).some((x) => x.agent.id === 'n3/e')).toBe(false);
  });
  it('puts what changed first when asked (the Changes pane), and keeps a plain order otherwise', () => {
    const g = folderChoices(fleet, true);
    expect(g[0]?.folders[0]?.agent.id).toBe('n2/c');
    expect(folderChoices(fleet, false)[0]?.project).toBe('app');
  });
  it('keys a folder by machine and workspace, an agent without one by itself', () => {
    expect(folderKey(agent({ id: 'x', workspaceRef: { node: 'n', id: 'w' } }))).toBe('n/w');
    expect(folderKey(agent({ id: 'x', workspaceRef: undefined }))).toBe('x');
  });
  it('opens beside the agent in focus: its terminal needs a terminal, the others a folder', () => {
    expect(toolTarget('changes', agent({}))).toBe('n1/a');
    expect(toolTarget('terminal', agent({ terminal: false }))).toBeUndefined();
    expect(toolTarget('terminal', agent({ workspaceRef: undefined }))).toBe('n1/a');
    expect(toolTarget('files', agent({ workspaceRef: undefined }))).toBeUndefined();
    expect(toolTarget('manager', undefined)).toBeUndefined();
  });
  it('with nothing in focus, goes to the one folder or terminal there is, and asks when there are several', () => {
    expect(soleTarget('changes', [agent({})])).toBe('n1/a');
    expect(soleTarget('manager', [agent({}), agent({ id: 'n1/b' })])).toBe('n1/a'); // two agents, one folder
    expect(soleTarget('files', fleet)).toBeUndefined();
    expect(soleTarget('terminal', [agent({}), agent({ id: 'n1/old', closed: true }), agent({ id: 'n1/chat', terminal: false })])).toBe('n1/a');
    expect(soleTarget('terminal', [agent({}), agent({ id: 'n1/b' })])).toBeUndefined();
    expect(soleTarget('changes', [])).toBeUndefined();
  });
});
