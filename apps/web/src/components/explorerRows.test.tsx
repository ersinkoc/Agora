// @vitest-environment jsdom
// #945: in the Agents list a helper row kept only "." of its name at 125 %: the name shared one truncating box with "started by …" and gave
// way to the badges, which never shrank. jsdom has no layout, so this pins the markup the fix relies on; scripts/e2e-helper-rows.mjs
// measures the real rows in Chromium.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { GroupBy } from '../net/fleet';
import { Explorer } from './Explorer';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const base = { machine: 'Laptop', machineId: 'laptop', workspace: 'proj', project: 'proj', state: 'working', preview: '', accent: 'var(--agent-amber)', shared: true, skipsPermissions: true } as const;
const fleet: AgentTileModel[] = [
  { ...base, id: 'laptop/a1', name: 'rm-acp · amber' },
  { ...base, id: 'laptop/a2', name: 'rm-helper · teal', parentId: 'laptop/a1', parentName: 'rm-acp · amber' },
  { ...base, id: 'laptop/a3', name: 'rm-helper · sky', parentId: 'laptop/a2', parentName: 'rm-helper · teal' },
];

describe('an Agents-list row (#945)', () => {
  it('keeps the name in its own element, ahead of "started by", with a floor of the row', () => {
    act(() => root.render(<Explorer by="machine" onBy={() => {}} spread={new Map()} fleet={fleet} machines={[]} open={[]} projects={[]} project="" onProject={() => {}} onToggle={() => {}} onCollapse={() => {}} onDismiss={() => {}} />));
    const rows = [...host.querySelectorAll<HTMLElement>('[data-agent-row]')];
    expect(rows).toHaveLength(3);
    for (const [i, row] of rows.entries()) {
      const a = fleet[i] as AgentTileModel;
      expect(row.classList.contains('@container')).toBe(true);
      const cell = row.querySelector('[data-agent-name-cell]') as HTMLElement;
      expect(cell.className).toContain('min-w-[40%]');
      const name = cell.querySelector('[data-agent-name]') as HTMLElement;
      expect(name.textContent).toBe(a.name);
      expect(name.className).toContain('flex-none'); // "started by …" gives way first
      expect(name.className).toContain('truncate');
      expect(row.title.startsWith(a.name)).toBe(true); // the full name when it is cut
      expect(row.title).toContain('Shared');
      if (a.parentName) expect(cell.textContent).toContain(`started by ${a.parentName}`);
      // An agent that acts without asking is one sign, a red bolt, never words; its meaning is its name, its tooltip and the row's title.
      expect(row.className).toContain('flex-wrap');
      const badge = row.querySelector('[data-no-ask]') as HTMLElement;
      expect(badge.getAttribute('role')).toBe('img');
      expect(badge.getAttribute('aria-label')).toMatch(/^Runs without asking: /);
      expect(badge.textContent).toBe('');
      expect(badge.className).toContain('text-risk');
      expect(badge.className).toContain('flex-none');
      expect(badge.hasAttribute('tabindex')).toBe(false); // inside the row's button, which stays the one stop
      expect(row.title).toContain('Runs without asking');
    }
  });
});

describe('large agent lists', () => {
  it('mounts nearby rows and retains a focused row as its chunk leaves the viewport', () => {
    type Visibility = (entries: { target: Element; isIntersecting: boolean }[]) => void;
    const observations = new Map<Element, Visibility>();
    vi.stubGlobal('IntersectionObserver', class {
      constructor(private readonly callback: Visibility) {}
      observe(element: Element) { observations.set(element, this.callback); }
      disconnect() {
        for (const [element, callback] of observations) {
          if (callback === this.callback) observations.delete(element);
        }
      }
    });
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });

    try {
      const many = Array.from({ length: 500 }, (_, i) => ({ ...base, id: `laptop/a-${i}`, name: `Agent ${i}` }));
      const render = (current?: string, items = many) => act(() => root.render(<Explorer by="machine" onBy={() => {}} spread={new Map()} fleet={items}
        machines={[]} open={[]} current={current} projects={[]} project="" onProject={() => {}} onToggle={() => {}}
        onCollapse={() => {}} onDismiss={() => {}} />));
      const rows = () => host.querySelectorAll<HTMLButtonElement>('[data-agent-row]');
      render();
      expect(host.querySelectorAll('[data-machine-id]')).toHaveLength(1);
      expect(rows().length).toBe(0);

      const [first, second] = [...observations.keys()];
      expect(first).toBeDefined();
      expect(second).toBeDefined();
      const show = (target: Element | undefined, isIntersecting: boolean) => {
        if (target) act(() => observations.get(target)?.([{ target, isIntersecting }]));
      };
      show(first, true);
      const row = rows()[0] as HTMLButtonElement;
      expect(row.dataset.agentRow).toBe('laptop/a-0');
      expect(rows().length).toBeLessThan(50);

      act(() => row.focus());
      show(first, false);
      expect(document.activeElement).toBe(row);
      expect(row.isConnected).toBe(true);

      act(() => row.blur());
      expect(row.isConnected).toBe(false);
      show(second, true);
      expect(rows().length).toBeGreaterThan(0);
      expect(rows().length).toBeLessThan(50);

      const crossing = host.querySelector<HTMLButtonElement>('[data-agent-row="laptop/a-12"]')!;
      act(() => crossing.focus());
      show(second, false);
      render(undefined, many.slice(1));
      expect(document.activeElement).toBe(crossing);
      expect(crossing.closest('li')?.getAttribute('aria-posinset')).toBe('12');
      expect(crossing.closest('li')?.getAttribute('aria-setsize')).toBe('499');

      render('laptop/a-499', many.slice(1));
      expect(host.querySelector('[data-agent-row="laptop/a-499"]')?.getAttribute('aria-current')).toBe('true');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});


const agents = (count: number) => Array.from({ length: count }, (_, i) => ({
  ...base, id: `laptop/b-${i}`, name: `Agent ${i}`, projectId: 'p1',
}));

function renderExplorer(items: AgentTileModel[], by: GroupBy = 'machine', current?: string) {
  act(() => root.render(<Explorer by={by} onBy={() => {}} spread={new Map()} fleet={items}
    machines={[]} open={[]} current={current} projects={[]} project="" onProject={() => {}} onToggle={() => {}}
    onCollapse={() => {}} onDismiss={() => {}} />));
}

it('keeps focus and a single accessible list after removing an earlier row', () => {
  const items = agents(20);
  renderExplorer(items);
  const button = host.querySelector<HTMLButtonElement>('[data-agent-row="laptop/b-5"]')!;
  act(() => button.focus());
  expect(document.activeElement).toBe(button);

  renderExplorer(items.slice(1));
  expect(document.activeElement).toBe(button);
  expect(host.querySelectorAll('nav ul')).toHaveLength(1);
  expect(button.closest('li')?.getAttribute('aria-posinset')).toBe('5');
  expect(button.closest('li')?.getAttribute('aria-setsize')).toBe('19');
});

it.each(['project', 'state'] as const)('keeps accessible %s groups while windowing rows', by => {
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} });
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  try {
    const items = agents(120).map((item, index) => index < 60 ? item : {
      ...item, projectId: 'p2', project: 'Other project', state: 'idle' as const,
    });
    renderExplorer(items, by, 'laptop/b-59');
    const groups = [...host.querySelectorAll('nav section[data-group]')];
    expect(groups).toHaveLength(2);
    expect(groups.map(group => group.getAttribute('data-group')).sort()).toEqual(by === 'project' ? ['p1', 'p2'] : ['idle', 'working']);
    for (const group of groups) expect(group.querySelectorAll('ul')).toHaveLength(1);
    const row = host.querySelector('[data-agent-row="laptop/b-59"]')?.closest('li');
    expect(row?.getAttribute('aria-posinset')).toBe('60');
    expect(row?.getAttribute('aria-setsize')).toBe('60');
    expect(host.querySelectorAll('[data-agent-row]')).toHaveLength(12);
  } finally {
    vi.unstubAllGlobals();
  }
});
