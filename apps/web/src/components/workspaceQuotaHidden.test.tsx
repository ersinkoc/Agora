// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { workspaceOf } from '../layout/engine';
import { LAYOUT_VERSION } from '../layout/tree';
import { Workspace } from '../layout/Workspace';
import { barePanels } from '../layout/testPanels';
import type { AgentTileModel } from '../model';
import type { QuotaObs } from '../net/quota';
import { QuotaProvider, QuotaSection } from './Quota';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = 1_000_000;
const edges = [['\u2069', '\\u{2069}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const;
const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id: 'n1/a1', name: 'codex', kind: 'codex', accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: '/work', state: 'working', preview: '', ...over,
});
const pane = (id: string, panel: string, bind?: { agent: string }) => ({ type: 'pane' as const, id, panel, ...(bind ? { bind } : {}) });
const one = workspaceOf({ version: LAYOUT_VERSION, root: pane('p-agent', 'agent', { agent: 'n1/a1' }) });
const split = workspaceOf({ version: LAYOUT_VERSION, root: { type: 'split' as const, direction: 'row' as const, ratios: [0.5, 0.5], children: [pane('p-agent', 'agent', { agent: 'n1/a1' }), pane('p-empty', 'empty')] } });

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); document.body.replaceChildren(); });

function draw(ws: typeof one, fleet: AgentTileModel[]) {
  act(() => root.render(<Workspace ws={ws} dispatch={() => {}} fleet={fleet} bindings={{ attention: new Map() }} narrow={false}
    {...barePanels(fleet)} welcome={null} />));
}

const quota = (names: ReadonlyMap<string, string>) => {
  const obs: QuotaObs[] = [
    { machineId: 'n1', tool: 'codex', window: 'five_hour', usedPercent: 20, resetsAtMs: NOW + 100_000, observedAtMs: NOW },
    { machineId: 'n2', tool: 'codex', window: 'five_hour', usedPercent: 30, resetsAtMs: NOW + 100_000, observedAtMs: NOW },
  ];
  return renderToStaticMarkup(<QuotaProvider obs={obs} machines={names} fixedNow={NOW}><QuotaSection fleet={[]} /></QuotaProvider>);
};

describe('workspace and quota labels make hidden characters visible', () => {
  it('writes hidden characters in the pane Show menu agent name as escapes', () => {
    draw(one, [agent({ name: 'co\u202edex' })]);
    act(() => host.querySelector<HTMLButtonElement>('.pane-bar button[aria-haspopup=menu]')?.click());
    const show = [...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent?.includes('Show'))!;
    act(() => show.click());
    expect(document.querySelector('[role=menu]')?.textContent).toContain('co\\u{202e}dex');
  });

  it('writes hidden characters in pane-bar machine names as escapes', () => {
    draw(one, [agent({ machine: 'lap\u3164top' })]);
    expect(host.querySelector('.pane-bar')?.textContent).toContain('lap\\u{3164}top');
  });

  it('writes hidden characters in empty-pane agent names as escapes', () => {
    draw(split, [agent({ name: 'co\u2066dex' })]);
    expect(host.querySelector('#pane-p-empty')?.textContent).toContain('co\\u{2066}dex');
  });

  it('writes hidden characters in empty-pane machine names as escapes', () => {
    draw(split, [agent({ machine: 'lap\u034ftop' })]);
    expect(host.querySelector('#pane-p-empty')?.textContent).toContain('lap\\u{34f}top');
  });

  it('writes hidden characters in visible and accessible quota machine labels as escapes', () => {
    const html = quota(new Map([['n1', 'lap\u200btop'], ['n2', 'server']]));
    expect(html).toContain('lap\\u{200b}top');
    expect(html).toContain('aria-label="lap\\u{200b}top, Codex, Five hours"');
  });
});

describe('workspace and quota label controls', () => {
  it('keeps an ordinary agent in the Show menu', () => {
    draw(one, [agent()]);
    act(() => host.querySelector<HTMLButtonElement>('.pane-bar button[aria-haspopup=menu]')?.click());
    const show = [...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent?.includes('Show'))!;
    act(() => show.click());
    expect(document.querySelector('[role=menu]')?.textContent).toContain('codex');
  });

  it('keeps an ordinary pane-bar machine and agent identity', () => {
    draw(one, [agent()]);
    expect(host.querySelector('.pane-bar')?.textContent).toContain('laptop');
    expect(host.querySelector('#pane-p-agent')?.getAttribute('data-agent')).toBe('n1/a1');
  });

  it('keeps an ordinary empty-pane agent name', () => {
    draw(split, [agent()]);
    expect(host.querySelector('#pane-p-empty')?.textContent).toContain('codex');
  });

  it('keeps an ordinary empty-pane machine name', () => {
    draw(split, [agent()]);
    expect(host.querySelector('#pane-p-empty')?.textContent).toContain('laptop');
  });

  it('keeps ordinary quota names and two separate progress bars', () => {
    const html = quota(new Map([['n1', 'laptop'], ['n2', 'server']]));
    expect(html).toContain('laptop');
    expect(html.match(/role="progressbar"/g)).toHaveLength(2);
  });
});

describe('workspace and quota label multi-edge verifier', () => {
  it('covers Show-menu agent names without changing pane binding', () => {
    for (const [char, escaped] of edges) {
      draw(one, [agent({ name: `co${char}dex` })]);
      const menuButton = host.querySelector<HTMLButtonElement>('.pane-bar button[aria-haspopup=menu]')!;
      act(() => menuButton.click());
      const show = [...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent?.includes('Show'))!;
      act(() => show.click());
      expect(document.querySelector('[role=menu]')?.textContent).toContain(`co${escaped}dex`);
      expect(host.querySelector('#pane-p-agent')?.getAttribute('data-agent')).toBe('n1/a1');
      act(() => menuButton.click());
    }
  });

  it('covers pane-bar machine names without changing pane binding', () => {
    for (const [char, escaped] of edges) {
      draw(one, [agent({ machine: `ma${char}chine` })]);
      expect(host.querySelector('.pane-bar')?.textContent).toContain(`ma${escaped}chine`);
      expect(host.querySelector('#pane-p-agent')?.getAttribute('data-agent')).toBe('n1/a1');
    }
  });

  it('covers picker agent names without changing the target pane', () => {
    for (const [char, escaped] of edges) {
      draw(split, [agent({ name: `co${char}dex` })]);
      expect(host.querySelector('#pane-p-empty')?.textContent).toContain(`co${escaped}dex`);
      expect(host.querySelector('#pane-p-empty')?.getAttribute('data-pane')).toBe('p-empty');
    }
  });

  it('covers picker machine names without changing agent identity', () => {
    for (const [char, escaped] of edges) {
      draw(split, [agent({ machine: `ma${char}chine` })]);
      expect(host.querySelector('#pane-p-empty')?.textContent).toContain(`ma${escaped}chine`);
      expect(host.querySelector('#pane-p-agent')?.getAttribute('data-agent')).toBe('n1/a1');
    }
  });

  it('covers quota machine labels without merging quota rows', () => {
    for (const [char, escaped] of edges) {
      const html = quota(new Map([['n1', `ma${char}chine`], ['n2', 'server']]));
      expect(html).toContain(`ma${escaped}chine`);
      expect(html).toContain(`aria-label="ma${escaped}chine, Codex, Five hours"`);
      expect(html.match(/role="progressbar"/g)).toHaveLength(2);
    }
  });
});
