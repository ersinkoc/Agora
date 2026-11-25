// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { resetGroupBy } from '../net/fleetPrefs';
import type { MachineModel } from '../net/view';
import { Turn } from './ChatEntry';
import { FleetConsole } from './FleetConsole';
import { StartAgent } from './StartAgent';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = 1_000_000;
const machine = (name: string): MachineModel => ({ id: 'n1', name, os: 'linux', online: true, agentsRunning: 1, lastSeenMs: NOW });
const fleet = (name: string) => [{ id: 'n1/a1', name: 'codex', kind: 'codex', accent: '#fff', machine: name, machineId: 'n1', workspace: '/work', state: 'working' as const, preview: '' }];
const fleetHtml = (name: string) => renderToStaticMarkup(<FleetConsole fleet={fleet(name)} machines={[machine(name)]} approvals={[]} link="live" now={NOW} onOpen={() => {}} onStart={() => {}} />);

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); resetGroupBy(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); document.body.replaceChildren(); });

function start(machineName: string, projectLabel = 'shop') {
  act(() => root.render(<StartAgent
    projects={[{ node: 'n1', projectId: 'p1', label: projectLabel, path: '/work/shop' }]}
    machines={[machine(machineName)]}
    loadKinds={() => new Promise(() => {})}
    inspect={async (_node, path) => ({ path, projectId: 'p1', gitRoot: path, branch: 'main', portable: false })}
    onStart={async () => undefined}
    onClose={() => {}}
  />));
}

describe('remaining owner-visible labels make hidden characters visible', () => {
  it('writes hidden characters in chat turn names as escapes', () => {
    const html = renderToStaticMarkup(<Turn name={'co\u202edex'} atMs={0}>reply</Turn>);
    expect(html).toContain('aria-label="co\\u{202e}dex"');
    expect(html).toContain('>co\\u{202e}dex<');
  });

  it('writes hidden characters in Fleet group headings as escapes', () => {
    expect(fleetHtml('lap\u3164top')).toContain('fleet-group__name">lap\\u{3164}top');
  });

  it('writes hidden characters in Fleet start-action names as escapes', () => {
    expect(fleetHtml('lap\u2066top')).toContain('aria-label="Start an agent on lap\\u{2066}top"');
  });

  it('writes hidden characters in Start Agent machine choices as escapes', () => {
    start('lap\u034ftop');
    const choice = document.querySelector('[role=radiogroup][aria-label="Machine"] [role=radio]') as HTMLElement;
    expect(choice.textContent).toContain('lap\\u{34f}top');
    expect(choice.title).toBe('lap\\u{34f}top');
  });

  it('writes hidden characters in Start Agent known-project labels as escapes', () => {
    start('laptop', 'sh\u200bop');
    expect(document.querySelector('[role=dialog]')?.textContent).toContain('sh\\u{200b}op');
  });
});

describe('remaining owner-visible label controls', () => {
  it('keeps an ordinary chat turn name', () => {
    const html = renderToStaticMarkup(<Turn name="codex" atMs={0}>reply</Turn>);
    expect(html).toContain('aria-label="codex"');
  });

  it('keeps an ordinary Fleet group heading', () => {
    expect(fleetHtml('laptop')).toContain('fleet-group__name">laptop');
  });

  it('keeps the Fleet start action bound to the raw machine id', () => {
    const html = fleetHtml('laptop');
    expect(html).toContain('aria-label="Start an agent on laptop"');
    expect(html).toContain('fleet-group');
  });

  it('keeps an ordinary Start Agent machine choice and raw value', () => {
    start('laptop');
    const choice = document.querySelector('[role=radiogroup][aria-label="Machine"] [role=radio]') as HTMLElement;
    expect(choice.textContent).toContain('laptop');
    expect(choice.getAttribute('aria-checked')).toBe('true');
  });

  it('keeps an ordinary known-project label and path', () => {
    start('laptop');
    const text = document.querySelector('[role=dialog]')?.textContent ?? '';
    expect(text).toContain('shop');
    expect(text).toContain('/work/shop');
  });
});

const verifierEdges = [
  ['\u2069', '\\u{2069}'],
  ['\u0085', '\\u{85}'],
  ['\u{f0000}', '\\u{f0000}'],
] as const;

describe('remaining owner-visible label verifier edges', () => {
  it('escapes other hidden characters in chat turn names without changing children', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<Turn name={`co${hidden}dex`} atMs={0}>reply</Turn>);
      expect(html).toContain(`aria-label="co${escaped}dex"`);
      expect(html).toContain(`>co${escaped}dex<`);
      expect(html).toContain('>reply</div>');
    }
  });

  it('escapes other hidden characters in Fleet group headings without losing the group', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = fleetHtml(`ma${hidden}chine`);
      expect(html).toContain(`fleet-group__name">ma${escaped}chine`);
      expect(html).toContain('fleet-group');
    }
  });

  it('escapes other hidden characters in Fleet start-action names without changing the action', () => {
    for (const [hidden, escaped] of verifierEdges) {
      let started: string | undefined = 'not-called';
      act(() => root.render(<FleetConsole
        fleet={fleet(`ma${hidden}chine`)}
        machines={[machine(`ma${hidden}chine`)]}
        approvals={[]}
        link="live"
        now={NOW}
        onOpen={() => {}}
        onStart={(id) => { started = id; }}
      />));
      const expected = `Start an agent on ma${escaped}chine`;
      const action = [...host.querySelectorAll('button')].find((button) => button.getAttribute('aria-label') === expected) as HTMLButtonElement;
      expect(action.title).toBe(expected);
      act(() => action.click());
      expect(started).toBe('n1');
    }
  });

  it('escapes other hidden characters in Start Agent machine choices without changing selection', () => {
    for (const [hidden, escaped] of verifierEdges) {
      start(`ma${hidden}chine`);
      const choice = document.querySelector('[role=radiogroup][aria-label="Machine"] [role=radio]') as HTMLElement;
      expect(choice.textContent).toContain(`ma${escaped}chine`);
      expect(choice.title).toBe(`ma${escaped}chine`);
      expect(choice.getAttribute('aria-checked')).toBe('true');
    }
  });

  it('escapes other hidden characters in known-project labels without changing the path', () => {
    for (const [hidden, escaped] of verifierEdges) {
      start('laptop', `pro${hidden}ject`);
      const text = document.querySelector('[role=dialog]')?.textContent ?? '';
      expect(text).toContain(`pro${escaped}ject`);
      expect(text).toContain('/work/shop');
    }
  });
});
