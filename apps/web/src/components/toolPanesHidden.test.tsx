// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { FolderBar, TerminalPane, ToolChooser } from './ToolPanes';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
vi.mock('../terminal/TerminalView', () => ({ TerminalView: ({ label }: { label: string }) => <div data-terminal-label={label} /> }));

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); document.body.querySelector('[role=menu]')?.remove(); });

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id: 'n1/a1', name: 'claude', accent: 'var(--agent-teal)', machine: 'laptop', machineId: 'n1', workspace: 'login', project: 'login', branch: 'main', state: 'working', preview: '',
  workspaceRef: { node: 'n1', id: 'w1' }, ...over,
});

describe('node-supplied tool-pane labels', () => {
  it('makes the folder bar and folder menu labels visible', () => {
    const shown = agent({ project: 'lo\u3164gin\u202e', machine: 'lap\u034ftop', branch: 'fi\ue000x' });
    act(() => root.render(<FolderBar agent={shown} fleet={[shown]} panel="files" onPick={() => {}} />));
    expect(host.textContent).toContain('lo\\u{3164}gin\\u{202e}');
    expect(host.textContent).toContain('lap\\u{34f}top');
    expect(host.textContent).toContain('fi\\u{e000}x');
    act(() => host.querySelector<HTMLButtonElement>('[aria-haspopup=menu]')?.click());
    expect(document.body.querySelector('[role=menu]')?.textContent).toContain('lo\\u{3164}gin\\u{202e} · lap\\u{34f}top');
  });

  it('verifies folder-bar labels across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u2066', '\\u{2066}'], ['\u0007', '\\u{7}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const shown = agent({ project: `pro${char}ject`, machine: `ma${char}chine`, branch: `br${char}anch` });
      act(() => root.render(<FolderBar agent={shown} fleet={[shown]} panel="files" onPick={() => {}} />));
      expect(host.textContent).toContain(`pro${escaped}ject`);
      expect(host.textContent).toContain(`ma${escaped}chine`);
      expect(host.textContent).toContain(`br${escaped}anch`);
    }
  });

  it('makes tool chooser project, agent, workspace, and machine labels visible', () => {
    const shown = agent({ name: 'clau\u3164de\u202e', project: 'lo\u2066gin', workspace: 'wo\u200brk', machine: 'lap\ue000top' });
    act(() => root.render(<ToolChooser panel="terminal" fleet={[shown]} onPick={() => {}} />));
    expect(host.textContent).toContain('clau\\u{3164}de\\u{202e}');
    expect(host.textContent).toContain('lap\\u{e000}top');
    act(() => root.render(<ToolChooser panel="files" fleet={[shown]} onPick={() => {}} />));
    expect(host.textContent).toContain('lo\\u{2066}gin');
    expect(host.textContent).toContain('wo\\u{200b}rk');
    expect(host.textContent).toContain('lap\\u{e000}top');
  });

  it('verifies chooser labels while selecting the raw agent identity', () => {
    const picked: string[] = [];
    for (const [char, escaped] of [['\u200b', '\\u{200b}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const shown = agent({ id: `raw${char}id`, name: `co${char}dex`, workspace: `wo${char}rk`, project: `pro${char}ject`, machine: `ma${char}chine` });
      act(() => root.render(<ToolChooser panel="terminal" fleet={[shown]} onPick={(id) => picked.push(id)} />));
      expect(host.textContent).toContain(`co${escaped}dex`);
      expect(host.textContent).toContain(`ma${escaped}chine`);
      act(() => host.querySelector<HTMLButtonElement>('[data-choice]')?.click());
      expect(picked.at(-1)).toBe(shown.id);
      act(() => root.render(<ToolChooser panel="files" fleet={[shown]} onPick={() => {}} />));
      expect(host.textContent).toContain(`pro${escaped}ject`);
      expect(host.textContent).toContain(`wo${escaped}rk`);
    }
  });

  it('makes terminal pane location and terminal accessible labels visible', async () => {
    const shown = agent({ name: 'clau\u3164de', machine: 'lap\u034ftop', project: 'lo\u2066gin', workspace: 'wo\u200brk', branch: 'fi\ue000x' });
    await act(async () => { root.render(<TerminalPane agent={shown} source={{ connect: () => ({ close() {}, sendInput() {}, resize() {} }) }} />); await Promise.resolve(); });
    expect(host.textContent).toContain('lap\\u{34f}top');
    expect(host.textContent).toContain('lo\\u{2066}gin');
    expect(host.textContent).toContain('fi\\u{e000}x');
    expect(host.querySelector('[data-terminal-label]')?.getAttribute('data-terminal-label')).toContain('clau\\u{3164}de');
  });

  it('verifies terminal labels across isolate, control, and supplementary private-use characters', async () => {
    for (const [char, escaped] of [['\u2069', '\\u{2069}'], ['\u0001', '\\u{1}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const shown = agent({ name: `co${char}dex`, machine: `ma${char}chine`, project: `pro${char}ject`, workspace: `wo${char}rk`, branch: `br${char}anch` });
      await act(async () => { root.render(<TerminalPane agent={shown} source={{ connect: () => ({ close() {}, sendInput() {}, resize() {} }) }} />); await Promise.resolve(); });
      expect(host.textContent).toContain(`ma${escaped}chine`);
      expect(host.textContent).toContain(`pro${escaped}ject`);
      expect(host.textContent).toContain(`br${escaped}anch`);
      expect(host.querySelector('[data-terminal-label]')?.getAttribute('data-terminal-label')).toContain(`co${escaped}dex`);
    }
  });
});
