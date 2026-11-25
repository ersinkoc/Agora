// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { resetGroupBy, saveGroupBy } from '../net/fleetPrefs';
import type { AttentionItem, ProjectSpread } from '../net/fleet';
import { Menu, type MenuItem } from '../layout/Menu';
import { AttentionQueue, FleetConsole } from './FleetConsole';
import { ProjectChip } from './ProjectMark';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = 1_000_000_000;
const noop = () => undefined;
const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id, name: `codex · ${id}`, accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: 'app', project: 'app', state: 'needs-you', preview: '', ...over,
});
const attention = (kind: string, machine: string): AttentionItem[] => Array.from({ length: 6 }, (_, i) => ({
  kind: 'agent' as const, key: `a${i}`, agent: agent(`a${i}`, { name: `${kind} · a${i}`, machine }), sinceMs: NOW - i,
}));
const queue = (kind: string, machine: string) => renderToStaticMarkup(
  <AttentionQueue items={attention(kind, machine)} machines={[]} now={NOW} onOpen={noop} />,
);
const chip = (machines: string[]) => renderToStaticMarkup(<ProjectChip spread={{ machines, agents: 2, changing: 0 }} />);

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  resetGroupBy();
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.querySelectorAll('[role=menu]').forEach((x) => x.remove());
  window.localStorage.clear();
  resetGroupBy();
});

const fleetFor = (first: string, second = 'server') => [
  agent('one', { name: 'codex · one', machine: first, machineId: 'n1', projectId: 'P', state: 'working' }),
  agent('two', { name: 'codex · two', machine: second, machineId: 'n2', projectId: 'P', state: 'working' }),
];
const mountProjectGroup = (first: string, second = 'server') => {
  saveGroupBy('project');
  act(() => root.render(<FleetConsole fleet={fleetFor(first, second)} machines={[]} approvals={[]} link="live" now={NOW} onOpen={noop} />));
  return host.querySelector<HTMLElement>('.fleet-group > p.muted.m-0.text-xs');
};
const openMenu = (label: string, items: MenuItem[]) => {
  act(() => root.render(<Menu key={label} label={label} items={items}><span>open</span></Menu>));
  const trigger = host.querySelector<HTMLButtonElement>('[aria-haspopup=menu]');
  act(() => trigger?.click());
  return { trigger, menu: document.body.querySelector<HTMLElement>('[role=menu]') };
};

describe('owner-visible labels at grouped fleet and menu boundaries', () => {
  it('F340 writes hidden characters in a collapsed agent-kind label out explicitly', () => {
    const html = queue('co\u2069dex', 'laptop');
    expect(html).toContain('6 co\\u{2069}dex agents ask for the same thing on laptop');
    expect(html).not.toContain('\u2069');
  });
  it('keeps an ordinary collapsed agent kind and its count unchanged', () => {
    const html = queue('codex', 'laptop');
    expect(html).toContain('6 codex agents ask for the same thing on laptop');
    expect(html.match(/fleet-ask--many/g)).toHaveLength(1);
  });

  it('F341 writes hidden characters in a collapsed machine label out explicitly', () => {
    const html = queue('codex', 'lap\u0085top');
    expect(html).toContain('6 codex agents ask for the same thing on lap\\u{85}top');
    expect(html).not.toContain('\u0085');
  });
  it('keeps an ordinary collapsed machine label unchanged', () => {
    expect(queue('codex', 'build-host')).toContain('6 codex agents ask for the same thing on build-host');
  });

  it('F342 writes hidden characters in a shared-project chip machine list out explicitly', () => {
    const html = chip(['lap\u2069top', 'server']);
    expect(html).toContain('Open on lap\\u{2069}top, server');
    expect(html).not.toContain('\u2069');
  });
  it('keeps an ordinary shared-project chip label and count unchanged', () => {
    const html = chip(['laptop', 'server']);
    expect(html).toContain('title="Same project · 2 machines. Open on laptop, server"');
    expect(html).toContain('Same project · 2 machines');
  });

  it('F343 writes hidden characters in a project group machine list out explicitly', () => {
    const line = mountProjectGroup('lap\u0085top');
    expect(line?.textContent).toBe('Open on lap\\u{85}top, server');
    expect(line?.textContent).not.toContain('\u0085');
  });
  it('keeps an ordinary project group machine list unchanged', () => {
    expect(mountProjectGroup('laptop')?.textContent).toBe('Open on laptop, server');
  });

  it('F344 writes hidden characters in generic menu labels and hints out explicitly', () => {
    const { trigger, menu } = openMenu('Me\u2069nu', [
      { label: 'It\u0085em', hint: 'Hi\u2069nt' },
      { label: 'Ti\u{f0000}le', checked: true, tile: <span>shape</span> },
    ]);
    expect(trigger?.getAttribute('aria-label')).toBe('Me\\u{2069}nu');
    expect(menu?.getAttribute('aria-label')).toBe('Me\\u{2069}nu');
    expect(menu?.textContent).toContain('It\\u{85}em');
    expect(menu?.textContent).toContain('Hi\\u{2069}nt');
    expect(menu?.textContent).toContain('Ti\\u{f0000}le');
    expect(menu?.querySelector('[data-tile]')?.getAttribute('aria-label')).toBe('Ti\\u{f0000}le');
  });
  it('keeps ordinary menu words and the raw action identity unchanged', () => {
    let chosen = 0;
    const { trigger, menu } = openMenu('Actions', [{ label: 'Run', hint: 'Now', run: () => { chosen++; } }]);
    expect(trigger?.getAttribute('title')).toBe('Actions');
    expect(menu?.textContent).toContain('Run');
    expect(menu?.textContent).toContain('Now');
    act(() => menu?.querySelector<HTMLButtonElement>('[role=menuitem]')?.click());
    expect(chosen).toBe(1);
  });
});

const EDGES = [['\u2069', '2069'], ['\u0085', '85'], ['\u{f0000}', 'f0000']] as const;

describe('independent hidden-character edge verification', () => {
  it('verifies collapsed agent kinds across isolate, control and private-use characters', () => {
    for (const [char, hex] of EDGES) {
      const html = queue(`co${char}dex`, 'laptop');
      expect(html).toContain(`6 co\\u{${hex}}dex agents ask for the same thing on laptop`);
      expect(html).not.toContain(char);
    }
  });
  it('verifies collapsed machine names across isolate, control and private-use characters', () => {
    for (const [char, hex] of EDGES) {
      const html = queue('codex', `lap${char}top`);
      expect(html).toContain(`6 codex agents ask for the same thing on lap\\u{${hex}}top`);
      expect(html).not.toContain(char);
    }
  });
  it('verifies project-chip machine names across isolate, control and private-use characters', () => {
    for (const [char, hex] of EDGES) {
      const html = chip([`lap${char}top`, 'server']);
      expect(html).toContain(`Open on lap\\u{${hex}}top, server`);
      expect(html).not.toContain(char);
    }
  });
  it('verifies project-group machine names across isolate, control and private-use characters', () => {
    for (const [char, hex] of EDGES) {
      const text = mountProjectGroup(`lap${char}top`)?.textContent ?? '';
      expect(text).toBe(`Open on lap\\u{${hex}}top, server`);
      expect(text).not.toContain(char);
    }
  });
  it('verifies menu labels and hints across isolate, control and private-use characters', () => {
    for (const [char, hex] of EDGES) {
      const { trigger, menu } = openMenu(`Me${char}nu`, [
        { label: `It${char}em`, hint: `Hi${char}nt` },
        { label: `Ti${char}le`, checked: true, tile: <span>shape</span> },
      ]);
      expect(trigger?.getAttribute('aria-label')).toBe(`Me\\u{${hex}}nu`);
      expect(menu?.getAttribute('aria-label')).toBe(`Me\\u{${hex}}nu`);
      expect(menu?.textContent).toContain(`It\\u{${hex}}em`);
      expect(menu?.textContent).toContain(`Hi\\u{${hex}}nt`);
      expect(menu?.textContent).toContain(`Ti\\u{${hex}}le`);
      expect(menu?.textContent).not.toContain(char);
    }
  });
});
