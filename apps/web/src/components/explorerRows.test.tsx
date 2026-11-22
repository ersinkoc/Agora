// @vitest-environment jsdom
// #945: in the Agents list a helper row kept only "." of its name at 125 %: the name shared one truncating box with "started by …" and gave
// way to the badges, which never shrank. jsdom has no layout, so this pins the markup the fix relies on; scripts/e2e-helper-rows.mjs
// measures the real rows in Chromium.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
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
