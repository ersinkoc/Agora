// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import { workspaceOf } from '../layout/engine';
import { forgetLoaded, saveWorkspace } from '../layout/store';
import { LAYOUT_VERSION } from '../layout/tree';
import type { AgentTileModel } from '../model';
import { demoSource } from '../terminal/source';
import { Console } from './Console';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
vi.mock('../terminal/TerminalView', () => ({ TerminalView: ({ label }: { label: string }) => <div data-terminal-label={label} /> }));

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id: 'n1/a1', name: 'codex', kind: 'codex', accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: '/work/login', project: '', branch: 'main', state: 'working', preview: '', terminal: true, ...over,
});
const edges = [['\u2069', '\\u{2069}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const;
const saved = workspaceOf({ version: LAYOUT_VERSION, root: { type: 'pane', id: 'p1', panel: 'agent', bind: { agent: 'n1/a1' } } });

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  window.localStorage.clear();
  forgetLoaded();
  saveWorkspace({ ws: saved, touched: true });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); document.body.replaceChildren(); window.localStorage.clear(); forgetLoaded(); });

function draw(a: AgentTileModel) {
  act(() => root.render(<Console agents={[a]} machines={[]} approvals={[]} messages={[]} terminalFor={() => demoSource('x')} onOpen={() => {}} link="live" />));
}
const crumbHeader = () => host.querySelector('[data-crumb=machine]')?.parentElement as HTMLElement;

describe('Console labels make hidden characters visible', () => {
  it('writes hidden characters in the location tooltip as escapes', () => {
    draw(agent({ machine: 'ma\u202echine', workspace: '/wo\u2066rk', branch: 'br\u034fanch' }));
    expect(crumbHeader().title).toBe('ma\\u{202e}chine · /wo\\u{2066}rk · br\\u{34f}anch');
  });

  it('writes hidden characters in the machine crumb as escapes', () => {
    draw(agent({ machine: 'lap\u3164top' }));
    expect(host.querySelector('[data-crumb=machine]')?.textContent).toContain('lap\\u{3164}top');
  });

  it('writes hidden characters in the workspace crumb as escapes', () => {
    draw(agent({ workspace: '/wo\u200brk' }));
    expect(crumbHeader().textContent).toContain('/wo\\u{200b}rk');
  });

  it('writes hidden characters in the branch crumb as escapes', () => {
    draw(agent({ branch: 'fi\ue000x' }));
    expect(host.querySelector('[data-crumb=branch]')?.textContent).toContain('fi\\u{e000}x');
  });

  it('writes hidden characters in the terminal label as escapes', () => {
    draw(agent({ name: 'co\u2069dex' }));
    expect(host.querySelector('[data-terminal-label]')?.getAttribute('data-terminal-label')).toBe('Terminal · co\\u{2069}dex');
  });
});

describe('Console label controls', () => {
  it('keeps an ordinary location tooltip', () => {
    draw(agent());
    expect(crumbHeader().title).toBe('laptop · /work/login · main');
  });

  it('keeps an ordinary machine crumb and raw agent binding', () => {
    draw(agent());
    expect(host.querySelector('[data-crumb=machine]')?.textContent).toContain('laptop');
    expect(host.querySelector('[data-pane=p1]')?.getAttribute('data-agent')).toBe('n1/a1');
  });

  it('keeps an ordinary workspace crumb', () => {
    draw(agent());
    expect(crumbHeader().textContent).toContain('/work/login');
  });

  it('keeps an ordinary branch crumb', () => {
    draw(agent());
    expect(host.querySelector('[data-crumb=branch]')?.textContent).toContain('main');
  });

  it('keeps an ordinary terminal label', () => {
    draw(agent());
    expect(host.querySelector('[data-terminal-label]')?.getAttribute('data-terminal-label')).toBe('Terminal · codex');
  });
});

describe('Console label multi-edge verifier', () => {
  it('covers location tooltips without changing pane identity', () => {
    for (const [char, escaped] of edges) {
      draw(agent({ machine: `ma${char}chine`, workspace: `/wo${char}rk`, branch: `br${char}anch` }));
      expect(crumbHeader().title).toBe(`ma${escaped}chine · /wo${escaped}rk · br${escaped}anch`);
      expect(host.querySelector('[data-pane=p1]')?.getAttribute('data-agent')).toBe('n1/a1');
    }
  });

  it('covers machine crumbs without changing pane identity', () => {
    for (const [char, escaped] of edges) {
      draw(agent({ machine: `ma${char}chine` }));
      expect(host.querySelector('[data-crumb=machine]')?.textContent).toContain(`ma${escaped}chine`);
      expect(host.querySelector('[data-pane=p1]')?.getAttribute('data-agent')).toBe('n1/a1');
    }
  });

  it('covers workspace crumbs without changing pane identity', () => {
    for (const [char, escaped] of edges) {
      draw(agent({ workspace: `/wo${char}rk` }));
      expect(crumbHeader().textContent).toContain(`/wo${escaped}rk`);
      expect(host.querySelector('[data-pane=p1]')?.getAttribute('data-agent')).toBe('n1/a1');
    }
  });

  it('covers branch crumbs without changing pane identity', () => {
    for (const [char, escaped] of edges) {
      draw(agent({ branch: `br${char}anch` }));
      expect(host.querySelector('[data-crumb=branch]')?.textContent).toContain(`br${escaped}anch`);
      expect(host.querySelector('[data-pane=p1]')?.getAttribute('data-agent')).toBe('n1/a1');
    }
  });

  it('covers terminal labels without changing the requested source identity', () => {
    for (const [char, escaped] of edges) {
      draw(agent({ name: `co${char}dex` }));
      expect(host.querySelector('[data-terminal-label]')?.getAttribute('data-terminal-label')).toBe(`Terminal · co${escaped}dex`);
      expect(host.querySelector('[data-pane=p1]')?.getAttribute('data-agent')).toBe('n1/a1');
    }
  });
});
