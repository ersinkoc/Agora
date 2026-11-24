// @vitest-environment jsdom
// Redesign PR 7 (ADR-0102): the Agents list, the pane tabs and bars, and the file rows. Who an agent is shows as its avatar wherever it is
// named; the row, tab or file in front is an accent tint, never a bar on one edge; a narrow pane keeps the avatar and the name (and below a
// threshold the avatar, with the name as its tooltip and accessible name). jsdom computes no layout and no Tailwind, so this pins the markup
// and the stylesheet rules; scripts/e2e-explorer-no-bars.mjs measures the real thing in Chromium.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { MachineModel } from '../net/view';
import { Workspace } from '../layout/Workspace';
import { reduce, workspaceOf, type Action, type Workspace as WorkspaceState } from '../layout/engine';
import { LAYOUT_VERSION } from '../layout/tree';
import { Explorer } from './Explorer';
import { FileManager } from './FileManager';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })); });
const base = { machine: 'Laptop', machineId: 'laptop', workspace: 'proj', project: 'proj', preview: '' } as const;
const fleet: AgentTileModel[] = [
  { ...base, id: 'laptop/a1', name: 'claude · amber', kind: 'claude', accent: 'var(--agent-amber)', state: 'needs-you', filesChanged: 6 },
  { ...base, id: 'laptop/a2', name: 'codex · teal', accent: 'var(--agent-teal)', state: 'working', skipsPermissions: true },
  { ...base, id: 'laptop/a3', name: 'gemini · rose', kind: 'gemini', accent: 'var(--agent-rose)', state: 'finished', closed: true, exitCode: 1, endedMs: Date.now() - 60_000 },
];
const machines: MachineModel[] = [
  { id: 'laptop', name: 'Laptop', os: 'linux', online: true, agentsRunning: 2, lastSeenMs: Date.now() },
  { id: 'win', name: 'win', os: 'windows', online: false, agentsRunning: 0, lastSeenMs: Date.now() - 2 * 3600_000 },
] as MachineModel[];
const noop = () => {};

// The Workspace dispatches actions; this applies them with the real engine.
function useStateRef(ws: WorkspaceState): [WorkspaceState, (a: Action) => void] {
  const [s, set] = useState(ws);
  return [s, (a) => set((cur) => reduce(cur, a))];
}

describe('an Agents-list row', () => {
  const render = () => act(() => root.render(<Explorer by="machine" onBy={noop} spread={new Map()} fleet={fleet} machines={machines} open={['laptop/a1']} current="laptop/a1"
    projects={[]} project="" onProject={noop} onToggle={noop} onCollapse={noop} onDismiss={noop} />));

  it('leads with the avatar and trails a neutral ±N, the bare red bolt and the state', () => {
    render();
    const row = host.querySelector<HTMLElement>('[data-agent-row="laptop/a1"]') as HTMLElement;
    const avatar = row.firstElementChild as HTMLElement;
    expect(avatar.classList.contains('kit-avatar')).toBe(true);
    expect(avatar.textContent?.startsWith('C')).toBe(true);
    expect(avatar.style.getPropertyValue('--avatar')).toBe('var(--agent-amber)');
    expect(avatar.querySelector('.kit-avatar__dot')).not.toBeNull(); // needs you
    const delta = row.querySelector('[data-files-changed]') as HTMLElement;
    expect(delta.textContent).toBe('±66 files changed'); // the visible ±6, and its words for a screen reader
    expect(delta.className).toContain('text-muted');
    expect(row.innerHTML).not.toContain('text-needs-you'); // a change count is never amber
    expect(row.lastElementChild?.classList.contains('kit-mark')).toBe(true); // the state's shape is last
    expect(row.getAttribute('aria-current')).toBe('true');
    expect(row.getAttribute('aria-pressed')).toBe('true');
    // A kind the node did not report comes from the name; the bolt is a bare icon (no chip), still one sign with its meaning as its name.
    const other = host.querySelector<HTMLElement>('[data-agent-row="laptop/a2"]') as HTMLElement;
    expect(other.querySelector('.kit-avatar')?.textContent).toBe('C');
    const bolt = other.querySelector('[data-no-ask]') as HTMLElement;
    expect(bolt.className).toContain('text-risk');
    expect(bolt.className).not.toContain('ide-chip');
    expect(bolt.getAttribute('aria-label')).toMatch(/^Runs without asking: /);
  });

  it('says an offline machine is "offline · 2 hr", keeps the words in its tooltip, and Closed has a ghost Clear', () => {
    render();
    const win = host.querySelector('[data-machine-id="win"]') as HTMLElement;
    expect(win.querySelector('[data-last-seen]')?.textContent).toBe('offline · 2 hr');
    expect(win.querySelector('button')?.title).toBe('Offline · last seen 2 hours ago');
    const clear = [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Clear') as HTMLButtonElement;
    expect(clear.classList.contains('kit-btn--ghost')).toBe(true);
    expect(clear.getAttribute('aria-label')).toBe('Clear closed');
    const closed = host.querySelector('section[aria-label="Closed"] li button') as HTMLElement;
    expect(closed.querySelector('.kit-avatar')?.textContent).toBe('G');
    expect(closed.querySelector('.kit-tone-danger')).not.toBeNull(); // ended with an error: the one red (ADR-0102)
  });
});

describe('pane tabs and bars', () => {
  const tabs = (ids: string[]) => ({ type: 'tabs' as const, active: 0, children: ids.map((id) => ({ type: 'pane' as const, id: `p-${id}`, panel: 'agent', bind: { agent: `laptop/${id}` } })) });
  function Harness({ ws }: { ws: WorkspaceState }) {
    const [state, setState] = useStateRef(ws);
    return <div style={{ position: 'relative', width: 600, height: 400 }}>
      <Workspace ws={state} dispatch={(a: Action) => setState(a)} fleet={fleet} bindings={{ attention: new Map() }} narrow={false}
        renderAgent={(a) => <p>{a.name}</p>} renderPanel={() => undefined} welcome={null} />
    </div>;
  }
  const split = workspaceOf({ version: LAYOUT_VERSION, root: { type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [tabs(['a1', 'a2']), { type: 'pane', id: 'p-solo', panel: 'agent', bind: { agent: 'laptop/a2' } }] } });

  it('names each agent tab with its avatar and its whole name, in its tooltip too', () => {
    act(() => root.render(<Harness ws={split} />));
    const tab = host.querySelector<HTMLElement>('#tab-p-a1') as HTMLElement;
    expect(tab.firstElementChild?.classList.contains('kit-avatar')).toBe(true);
    expect(tab.querySelector('[data-pane-name]')?.textContent).toBe('claude · amber');
    expect(tab.title).toBe('claude · amber');
    expect(tab.querySelector('.kit-mark')).not.toBeNull();
    // The bar is measured on its own (app.css `.pane-bar`), and its split and maximize can fold into ⋯ in a narrow pane.
    const bar = tab.closest('.pane-bar') as HTMLElement;
    expect(bar.querySelectorAll('[data-pane-tool="split"]')).toHaveLength(2);
    // A pane without tabs keeps the avatar (its tooltip is the name) and the name.
    const solo = [...host.querySelectorAll<HTMLElement>('.pane-bar')].find((b) => !b.querySelector('[role=tablist]')) as HTMLElement;
    expect(solo.querySelector('[title="codex · teal"] .kit-avatar')).not.toBeNull();
    expect(solo.querySelector('[data-pane-name]')?.textContent).toBe('codex · teal');
    expect(solo.querySelector('[data-no-ask]')?.className).not.toContain('ide-chip');
  });

  it('still moves between tabs with the arrow keys, Home and End', () => {
    act(() => root.render(<Harness ws={split} />));
    const first = host.querySelector<HTMLElement>('#tab-p-a1') as HTMLElement;
    expect(first.getAttribute('aria-selected')).toBe('true');
    key(first, 'ArrowRight');
    expect(host.querySelector('#tab-p-a2')?.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement?.id).toBe('tab-p-a2');
    key(host.querySelector('#tab-p-a2') as HTMLElement, 'Home');
    expect(host.querySelector('#tab-p-a1')?.getAttribute('aria-selected')).toBe('true');
  });
});

describe('file rows', () => {
  const files = { list: async (p: string) => (p === '' ? [{ name: 'src', isDir: true, size: 0 }, { name: 'a.md', isDir: false, size: 1 }, { name: 'b.md', isDir: false, size: 2 }] : []), read: async () => new Uint8Array() };
  it('move by keyboard and mark the chosen one by aria-selected (a tint), not a bar', async () => {
    act(() => root.render(<FileManager files={files} rootName="app" />));
    await act(async () => { await new Promise((r) => setTimeout(r, 0)); });
    const rows = [...host.querySelectorAll<HTMLElement>('[role=row][data-path]')];
    expect(rows.length).toBe(3);
    for (const r of rows) expect(r.className).toContain('fm-row');
    act(() => rows[0]?.focus());
    key(rows[0] as HTMLElement, 'ArrowDown');
    expect((document.activeElement as HTMLElement).dataset.path).toBe('a.md');
    const tree = host.querySelector('[role=treeitem]') as HTMLElement;
    expect(tree.className).toContain('fm-row');
    expect(tree.getAttribute('aria-selected')).toBe('true');
    expect(host.innerHTML).not.toMatch(/inset_2px/);
  });
});

describe('no bar on one side (the stylesheet and the components)', () => {
  const src = join(__dirname, '..');
  const css = readFileSync(join(src, 'design/app.css'), 'utf8');
  const rule = (sel: string) => css.split('\n').filter((l) => l.trimStart().startsWith(sel)).join('\n');

  it('draws the row, tab, rail icon and file row in front as a tint or a ring, never an inset offset to one side', () => {
    for (const sel of [".ide-row[aria-current='true']", ".rail__btn[aria-selected='true']", ".fm-row[aria-selected='true']", '.fm-row:focus-visible']) {
      const r = rule(sel);
      expect(r, sel).not.toBe('');
      expect(r, sel).not.toMatch(/inset\s+-?\d+(\.\d+)?px\s+0\s/); // inset X 0 …: a bar on the start or end edge
    }
    expect(rule(".ide-row[aria-current='true']")).toContain('var(--tint-accent)');
    expect(rule('.fm-row:focus-visible')).toContain('inset 0 0 0 2px var(--accent)'); // a ring on all four sides
    // The only edge bars left are content conventions: the diff's 2 px line bar (visual-language.md §2).
    const sideBars = css.split('\n').filter((l) => /inset\s+-?[1-9]\d*(\.\d+)?px\s+0\s+0/.test(l));
    expect(sideBars.every((l) => l.includes('.diff-bar-'))).toBe(true);
  });

  it('has no inset start bar in any component', () => {
    const walk = (dir: string): string[] => readdirSync(dir).flatMap((f) => { const p = join(dir, f); return statSync(p).isDirectory() ? walk(p) : p.endsWith('.tsx') && !p.endsWith('.test.tsx') ? [p] : []; });
    const offenders = walk(src).filter((f) => /shadow-\[inset_-?\d+px_0/.test(readFileSync(f, 'utf8')));
    expect(offenders).toEqual([]);
  });

  it('folds a narrow pane by its own width and keeps the avatar when the name goes', () => {
    expect(css).toMatch(/\.pane-bar \{ container: pane-bar \/ inline-size; \}/);
    expect(css).toMatch(/@container pane-bar \(max-width: 26rem\) \{ \.pane-bar \[data-pane-tool='split'\] \{ display: none; \} \}/);
    expect(css).toMatch(/@container tab-strip \(max-width: 11rem\) \{[^}]*\{ min-width: auto; \}\s*\.ide-tab \[data-pane-name\] \{ @apply sr-only; \}/);
    expect(css).not.toMatch(/\[data-pane-name\] \{ display: none/); // hidden from sight only: it stays the tab's accessible name
  });
});
