// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import type { FileItem, WorkspaceFiles } from '../net/files';
import { FileManager } from './FileManager';
import { ChangesPane, FilesPane, ManagerPane, ToolChooser } from './ToolPanes';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const TREE: Record<string, FileItem[]> = {
  '': [{ name: 'src', isDir: true, size: 0 }, { name: 'docs', isDir: true, size: 0 }, { name: 'README.md', isDir: false, size: 120 }, { name: 'big.bin', isDir: false, size: 900_000 }, { name: '.env', isDir: false, size: 9, secret: true }, { name: 'Cargo.toml', isDir: false, size: 40 }],
  src: [{ name: 'main.rs', isDir: false, size: 30 }, { name: 'lib.rs', isDir: false, size: 10 }, { name: 'util', isDir: true, size: 0 }],
  'src/util': [{ name: 'x.rs', isDir: false, size: 1 }],
  docs: [],
};
const CONTENT: Record<string, Uint8Array> = {
  'README.md': new TextEncoder().encode('# hello\nworld\n'),
  'src/main.rs': new TextEncoder().encode('fn main() {}\n'),
  'big.bin': new Uint8Array([0, 1, 2]),
};
const reads: string[] = [];
const files = (over: Partial<WorkspaceFiles> = {}): WorkspaceFiles => ({
  list: async (p) => { const l = TREE[p]; if (!l) throw new Error('no'); return l; },
  read: async (p) => { reads.push(p); const c = CONTENT[p]; if (!c) throw new Error('no'); return c; },
  ...over,
});
const flush = async () => { await act(async () => { await new Promise((r) => setTimeout(r, 0)); }); };
let mounts = 0;
const mount = async (f: WorkspaceFiles = files(), extra: { onTerminal?: () => void; terminalWhy?: string } = {}) => { reads.length = 0; act(() => root.render(<FileManager key={++mounts} files={f} rootName="app" {...extra} />)); await flush(); };
const names = () => [...host.querySelectorAll('[role=row][data-path] [role=gridcell]:first-child')].map((c) => c.textContent);
const key = (el: Element, k: string, init: KeyboardEventInit = {}) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, ...init })); });
const row = (path: string) => host.querySelector<HTMLElement>(`[role=row][data-path="${path}"]`) as HTMLElement;
const click = (el: Element) => act(() => { (el as HTMLElement).click(); });

describe('the file manager list', () => {
  it('makes listed file names visible', async () => {
    await mount(files({ list: async () => [{ name: 'Grü\u3164ße\u2800.ts', isDir: false, size: 1 }] }));
    const shown = host.textContent ?? '';
    expect([...shown].some((c) => ['\u3164', '\u2800'].includes(c))).toBe(false);
    expect(shown).toContain('Grü\\u{3164}ße\\u{2800}.ts');
  });
  it('verifies multilingual display while preserving the raw row path', async () => {
    const raw = '漢\u202e字\u3164.rs';
    await mount(files({ list: async () => [{ name: raw, isDir: false, size: 1 }] }));
    expect(host.textContent).toContain('漢\\u{202e}字\\u{3164}.rs');
    expect(host.querySelector('[role=row][data-path]')?.getAttribute('data-path')).toBe(raw);
  });

  it('lists a folder with folders first, kind and size, and a count', async () => {
    await mount();
    expect(names()).toEqual(['docs', 'src', '.env', 'big.bin', 'Cargo.toml', 'README.md']);
    expect(host.textContent).toContain('120 B');
    expect(host.textContent).toContain('Private file');
    expect(host.textContent).toContain('6 items');
    expect(host.querySelector('[role=grid]')?.getAttribute('aria-rowcount')).toBe('7');
  });
  it('sorts by a column, again to turn it round, and says so to a screen reader', async () => {
    await mount();
    const size = [...host.querySelectorAll('[role=columnheader] button')].find((b) => b.textContent === 'Size') as HTMLElement;
    click(size);
    expect(names().slice(2)).toEqual(['.env', 'Cargo.toml', 'README.md', 'big.bin']);
    expect(host.querySelector('[role=columnheader][aria-sort=ascending]')?.textContent).toBe('Size');
    click(size);
    expect(names().slice(2)).toEqual(['big.bin', 'README.md', 'Cargo.toml', '.env']);
    expect(host.querySelector('[role=columnheader][aria-sort=descending]')).not.toBeNull();
  });
  it('filters by name, says when nothing matches, and Escape clears the filter', async () => {
    await mount();
    const input = host.querySelector<HTMLInputElement>('#fm-filter') as HTMLInputElement;
    const set = (v: string) => act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, v); input.dispatchEvent(new Event('input', { bubbles: true })); });
    set('read');
    expect(names()).toEqual(['README.md']);
    set('zzz');
    expect(host.textContent).toContain(t('files-no-match'));
    key(row('README.md') ?? host.querySelector('[role=grid]') as Element, 'Escape');
    set('');
    expect(names()).toHaveLength(6);
  });
  it('opens a folder, shows the breadcrumb, goes up, and the tree follows', async () => {
    await mount();
    click(row('src'));
    await flush();
    expect(names()).toEqual(['util', 'lib.rs', 'main.rs']);
    expect(host.querySelector('nav')?.textContent).toContain('src');
    expect(host.querySelector('[role=treeitem][aria-selected=true]')?.getAttribute('data-path')).toBe('src');
    click(host.querySelector('button[aria-label="Parent folder"]') as Element);
    await flush();
    expect(host.querySelector('nav [aria-current=page]')?.textContent).toBe('app');
    expect(names()).toHaveLength(6);
  });
  it('says an empty folder is empty, a failing one failed (with a way to retry)', async () => {
    await mount();
    click(row('docs'));
    await flush();
    expect(host.textContent).toContain(t('fm-empty'));
    await mount(files({ list: async () => { throw new Error('down'); } }));
    await act(async () => { await new Promise((r) => setTimeout(r, 3200)); });
    expect(host.querySelector('[role=alert]')?.textContent).toContain(t('files-failed'));
  }, 10_000);
  it('shows the loading state before the machine answers', async () => {
    let release: (l: FileItem[]) => void = () => {};
    await mount(files({ list: () => new Promise((r) => { release = r; }) }));
    expect(host.querySelector('[role=grid]')?.getAttribute('aria-busy')).toBe('true');
    await act(async () => { release([{ name: 'a', isDir: false, size: 1 }]); });
    expect(host.querySelector('[role=grid]')?.getAttribute('aria-busy')).toBe('false');
  });
});

describe('the file manager keyboard', () => {
  it('arrows move between rows (one Tab stop), Enter opens, Left or Backspace goes up, Home and End jump', async () => {
    await mount();
    const stops = () => [...host.querySelectorAll('[role=row][data-path][tabindex="0"]')].length;
    expect(stops()).toBe(1);
    row('docs').focus();
    key(row('docs'), 'ArrowDown');
    expect(document.activeElement).toBe(row('src'));
    key(row('src'), 'End');
    expect(document.activeElement).toBe(row('README.md'));
    key(row('README.md'), 'Home');
    expect(document.activeElement).toBe(row('docs'));
    row('src').focus();
    key(row('src'), 'Enter');
    await flush();
    expect(names()[0]).toBe('util');
    expect(document.activeElement).toBe(row('src/util'));
    key(row('src/util'), 'Backspace');
    await flush();
    expect(names()).toHaveLength(6);
  });
  it('the tree is one Tab stop and opens and closes folders with Right and Left', async () => {
    await mount();
    const items = () => [...host.querySelectorAll<HTMLElement>('[role=treeitem]')];
    expect(items().filter((i) => i.tabIndex === 0)).toHaveLength(1);
    const rootItem = items()[0] as HTMLElement;
    rootItem.focus();
    key(rootItem, 'ArrowRight'); // already open: into the first child
    expect(document.activeElement?.getAttribute('data-path')).toBe('docs');
    key(document.activeElement as Element, 'ArrowLeft'); // a leaf-ish folder: out to its parent
    expect(document.activeElement?.getAttribute('data-path')).toBe('');
  });
  it('Escape closes the preview and returns to the list', async () => {
    await mount();
    click(row('README.md'));
    await flush();
    expect(host.querySelector('section[aria-label=Preview]')).not.toBeNull();
    key(host.querySelector('[role=group]') as Element, 'Escape');
    expect(host.querySelector('section[aria-label=Preview]')).toBeNull();
  });
  it('keeps the window\'s one-letter shortcuts out (n is not the next agent here)', async () => {
    await mount();
    let seen = false;
    const on = () => { seen = true; };
    document.addEventListener('keydown', on);
    key(row('src'), 'n');
    document.removeEventListener('keydown', on);
    expect(seen).toBe(false);
  });
});

describe('a folder of thousands', () => {
  const many: FileItem[] = Array.from({ length: 5000 }, (_, i) => ({ name: `file${i}.txt`, isDir: false, size: i }));
  const rendered = () => host.querySelectorAll('[role=row][data-path]').length;
  it('draws only the rows in view, keeps the full count for assistive technology, and sorts quickly', async () => {
    await mount(files({ list: async () => many }));
    expect(rendered()).toBeLessThan(120);
    expect(host.querySelector('[role=grid]')?.getAttribute('aria-rowcount')).toBe('5001');
    expect(host.querySelector('[role=row][data-path="file0.txt"]')?.getAttribute('aria-rowindex')).toBe('2');
    expect(host.textContent).toContain('5,000 items');
    const size = [...host.querySelectorAll('[role=columnheader] button')].find((b) => b.textContent === 'Size') as HTMLElement;
    const t0 = performance.now();
    click(size); click(size);
    expect(performance.now() - t0).toBeLessThan(1500);
    expect(rendered()).toBeLessThan(120);
  });
  it('End goes to the last row, which is drawn and focused; Home comes back; one row is the Tab stop throughout', async () => {
    await mount(files({ list: async () => many }));
    const stops = () => host.querySelectorAll('[role=row][data-path][tabindex="0"]').length;
    const firstRow = host.querySelector<HTMLElement>('[role=row][data-path]') as HTMLElement;
    firstRow.focus();
    key(firstRow, 'End');
    await flush();
    expect(document.activeElement?.getAttribute('data-path')).toBe('file4999.txt');
    expect(document.activeElement?.getAttribute('aria-rowindex')).toBe('5001');
    expect(stops()).toBe(1);
    key(document.activeElement as Element, 'Home');
    await flush();
    expect(document.activeElement?.getAttribute('aria-rowindex')).toBe('2');
    expect(rendered()).toBeLessThan(120);
  });
});

describe('leaving', () => {
  it('stops retrying a listing that failed once the manager is gone', async () => {
    let calls = 0;
    await mount(files({ list: async () => { calls++; throw new Error('down'); } }));
    const before = calls;
    act(() => root.render(null));
    await act(async () => { await new Promise((r) => setTimeout(r, 1300)); });
    expect(calls).toBe(before);
  });
});

describe('the preview', () => {
  it('makes the code-region file label visible while reading the raw path', async () => {
    const raw = 'Grü\u3164ße\u2800.ts';
    const read = async (p: string) => { reads.push(p); return new TextEncoder().encode('x'); };
    await mount(files({ list: async () => [{ name: raw, isDir: false, size: 1 }], read }));
    click(row(raw)); await flush();
    const region = host.querySelector('[role=region]') as HTMLElement;
    expect([...region.getAttribute('aria-label') ?? ''].some((c) => ['\u3164', '\u2800'].includes(c))).toBe(false);
    expect(region.getAttribute('aria-label')).toBe('Grü\\u{3164}ße\\u{2800}.ts');
    expect(reads).toContain(raw);
  });
  it('verifies code-region labels across bidi and control characters without changing reads', async () => {
    const raw = 'İş\u202e\u0001.rs';
    const read = async (p: string) => { reads.push(p); return new TextEncoder().encode('x'); };
    await mount(files({ list: async () => [{ name: raw, isDir: false, size: 1 }], read }));
    const target = [...host.querySelectorAll<HTMLElement>('[role=row][data-path]')].find((el) => el.dataset.path === raw);
    expect(target).toBeDefined();
    click(target as HTMLElement); await flush();
    expect(host.querySelector('[role=region]')?.getAttribute('aria-label')).toBe('İş\\u{202e}\\u{1}.rs');
    expect(reads).toEqual([raw]);
  });

  it('reads a text file once, shows its lines and size, and closes', async () => {
    await mount();
    click(row('README.md'));
    await flush();
    expect(reads).toEqual(['README.md']);
    const p = host.querySelector('section[aria-label=Preview]') as HTMLElement;
    expect(p.textContent).toContain('README.md');
    expect(p.textContent).toContain('hello');
    click(p.querySelector('button[aria-label="Close the preview"]') as Element);
    expect(host.querySelector('section[aria-label=Preview]')).toBeNull();
  });
  it('says a file that is not text is not shown, and never reads a private file at all', async () => {
    await mount();
    click(row('big.bin'));
    await flush();
    expect(host.textContent).toContain(t('fm-binary'));
    click(row('.env'));
    await flush();
    expect(reads).toEqual(['big.bin']);
    expect(host.textContent).toContain(t('files-secret'));
  });
  it('says a file that cannot be read failed, and drops a read that finished after another file was chosen', async () => {
    let finish: (b: Uint8Array) => void = () => {};
    await mount(files({ read: (p) => (p === 'README.md' ? new Promise<Uint8Array>((r) => { finish = r; }) : Promise.reject(new Error('too large'))) }));
    click(row('README.md'));
    click(row('Cargo.toml'));
    await flush();
    await act(async () => { finish(new TextEncoder().encode('late')); });
    const p = host.querySelector('section[aria-label=Preview]') as HTMLElement;
    expect(p.textContent).not.toContain('late');
    expect(p.querySelector('[role=alert]')).not.toBeNull();
  });
});

describe('the actions', () => {
  it('shows what would change files switched off, each with the reason, and never acts', async () => {
    await mount();
    const off = ['Rename', 'Move', 'Delete', 'Upload'].map((n) => host.querySelector<HTMLElement>(`button[aria-label=${n}]`) as HTMLElement);
    for (const b of off) {
      expect(b.getAttribute('aria-disabled')).toBe('true');
      expect(b.getAttribute('title')).toContain(t('fm-write-why'));
      expect(b.getAttribute('aria-describedby')).toBe('fm-why');
      click(b);
    }
    expect(host.querySelector('#fm-why')?.textContent).toBe(t('fm-write-why'));
    // A tap shows the reason on the page too (a tooltip does not exist on a touch screen).
    expect(host.querySelector('[role=status]')?.textContent).toBe(t('fm-write-why'));
    expect(reads).toEqual([]);
  });
  it('offers Download for a chosen file only, and Agent terminal at the folder\'s top only', async () => {
    let opened = 0;
    await mount(files(), { onTerminal: () => { opened++; } });
    const dl = () => host.querySelector<HTMLButtonElement>('button[aria-label=Download]') as HTMLButtonElement;
    expect(dl().disabled).toBe(true);
    click(row('README.md'));
    await flush();
    expect(dl().disabled).toBe(false);
    click(row('.env'));
    await flush();
    expect(dl().disabled).toBe(true); // a private file is never downloaded
    const term = [...host.querySelectorAll('[role=toolbar] button')].find((b) => b.textContent === 'Agent terminal') as HTMLElement;
    click(term);
    expect(opened).toBe(1);
    click(row('src'));
    await flush();
    expect(term.getAttribute('aria-disabled')).toBe('true');
    expect(term.getAttribute('title')).toBe(t('fm-terminal-subfolder'));
    click(term);
    expect(opened).toBe(1);
  });
  it('says why there is no terminal when the agent has none', async () => {
    await mount(files(), { terminalWhy: 'fm-terminal-chat' });
    const term = [...host.querySelectorAll('[role=toolbar] button')].find((b) => b.textContent === 'Agent terminal') as HTMLElement;
    expect(term.getAttribute('aria-disabled')).toBe('true');
    expect(term.getAttribute('title')).toBe(t('fm-terminal-chat'));
  });
});

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n1/a', name: 'claude · amber', accent: '#fff', machine: 'laptop', workspace: 'app', project: 'app', state: 'idle', preview: '', workspaceRef: { node: 'n1', id: 'w' }, ...over });

describe('the tool panes', () => {
  it('a chooser lists folders grouped by project, picks one, and moves with the arrows', () => {
    const picked: string[] = [];
    const fleet = [agent(), agent({ id: 'n2/b', machine: 'vps', project: 'api', workspace: 'api', workspaceRef: { node: 'n2', id: 'x' }, filesChanged: 3 })];
    act(() => root.render(<ToolChooser panel="changes" fleet={fleet} onPick={(id) => picked.push(id)} />));
    expect([...host.querySelectorAll('h3')].map((h) => h.textContent)).toEqual(['api', 'app']); // changes first
    const buttons = [...host.querySelectorAll<HTMLElement>('button[data-choice]')];
    buttons[0]?.focus();
    key(buttons[0] as Element, 'ArrowDown');
    expect(document.activeElement).toBe(buttons[1]);
    click(buttons[1] as Element);
    expect(picked).toEqual(['n1/a']);
  });
  it('a chooser with nothing to choose says so and what makes something appear', () => {
    act(() => root.render(<ToolChooser panel="files" fleet={[]} onPick={() => {}} />));
    expect(host.textContent).toContain(t('pane-no-folders-title'));
    expect(host.textContent).toContain(t('pane-no-folders-hint'));
  });
  it('a terminal chooser lists the live terminals and offers a new one when it can', () => {
    let fresh = 0;
    const fleet = [agent(), agent({ id: 'n1/chat', name: 'chat', terminal: false }), agent({ id: 'n1/done', name: 'done', closed: true })];
    act(() => root.render(<ToolChooser panel="terminal" fleet={fleet} onPick={() => {}} onNewTerminal={() => { fresh++; }} />));
    expect([...host.querySelectorAll('button[data-choice]')].map((b) => b.textContent)).toEqual(['claude · amberlaptop']);
    click([...host.querySelectorAll('button')].find((b) => b.textContent === 'New terminal') as Element);
    expect(fresh).toBe(1);
  });
  it('each pane says honestly when it cannot show anything', () => {
    const noop = () => {};
    act(() => root.render(<><ChangesPane agent={agent()} fleet={[]} onPick={noop} /><FilesPane agent={agent()} fleet={[]} onPick={noop} /><ManagerPane agent={agent()} fleet={[]} onPick={noop} /></>));
    expect(host.querySelectorAll('[role=status]')).toHaveLength(3);
    expect(host.textContent).toContain(t('pane-offline-hint'));
    act(() => root.render(<ManagerPane agent={agent({ workspaceRef: undefined })} fleet={[]} filesOf={() => files()} onPick={noop} />));
    expect(host.textContent).toContain(t('pane-no-folder-title'));
  });
  it('the manager pane switches folder from its bar and opens the agent\'s terminal only when there is one', async () => {
    const picked: string[] = [];
    const fleet = [agent(), agent({ id: 'n2/b', machine: 'vps', project: 'api', workspace: 'api', workspaceRef: { node: 'n2', id: 'x' } })];
    act(() => root.render(<ManagerPane agent={fleet[0] as AgentTileModel} fleet={fleet} filesOf={() => files()} onPick={(id) => picked.push(id)} onTerminal={() => {}} />));
    await flush();
    click(host.querySelector('[data-folder-bar] button[aria-label="Show another folder"]') as Element);
    const items = [...document.querySelectorAll('[role=menuitemradio]')];
    expect(items.map((i) => i.textContent)).toEqual(['api · vps', 'app · laptop']);
    click(items[0] as Element);
    expect(picked).toEqual(['n2/b']);
  });
});

describe('words', () => {
  it('every pane message exists in Turkish too', () => {
    const keys = ['fm-write-why', 'fm-terminal-subfolder', 'pane-offline-hint', 'palette-open-manager', 'fm-kind-private', 'layout-panel-manager', 'layout-tool-of'];
    setLanguage('tr');
    for (const k of keys) expect(t(k, { what: 'a', name: 'b' }), k).not.toBe(k);
    expect(t('fm-count', { count: 2 })).toBe('2 öğe');
    expect(t('fm-write-why')).not.toBe('Changing files needs your OK on the machine, and this window cannot ask for it yet.');
  });
});
