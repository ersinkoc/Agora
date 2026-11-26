// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { CommitItem } from '../net/history';
import type { MachineModel } from '../net/view';
import { AttentionStrip } from './AttentionStrip';
import { HistoryDialog } from './HistoryDialog';
import { MachineName } from './machineFacts';
import { NetworkView } from './NetworkView';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = 1_000_000;
const edges = [['\u2069', '\\u{2069}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const;
const agent = (name: string): AgentTileModel => ({
  id: 'n1/a1', name, kind: 'codex', accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: '/work', state: 'needs-you', preview: '',
});
const machine = (name: string): MachineModel => ({ id: 'n1', name, os: 'linux', online: true, agentsRunning: 1, lastSeenMs: NOW });

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); document.body.replaceChildren(); });

async function history(commit: CommitItem) {
  await act(async () => {
    root.render(<HistoryDialog agent={agent('codex')} load={async () => [commit]} onClose={() => {}} />);
  });
  return document.querySelector('[role=dialog]')?.textContent ?? '';
}

describe('owner-visible labels make hidden characters visible', () => {
  it('writes hidden characters in a commit subject as escapes', async () => {
    expect(await history({ id: '0123456789', subject: 'fix‮hidden', author: 'Ada', atMs: 0 })).toContain('fix\\u{202e}hidden');
  });

  it('writes hidden characters in a commit author as escapes', async () => {
    expect(await history({ id: '0123456789', subject: 'Fix', author: 'Aㅤda', atMs: 0 })).toContain('A\\u{3164}da');
  });

  it('writes hidden characters in attention-strip agent names as escapes', () => {
    expect(renderToStaticMarkup(<AttentionStrip agents={[agent('co⁦dex')]} />)).toContain('co\\u{2066}dex');
  });

  it('writes hidden characters in machine names as escapes', () => {
    expect(renderToStaticMarkup(<MachineName m={machine('lap͏top')} />)).toContain('lap\\u{34f}top');
  });

  it('writes hidden characters in machine-card agent names as escapes', () => {
    expect(renderToStaticMarkup(<NetworkView now={NOW} machines={[machine('laptop')]} agents={[agent('co​dex')]} hubName="hub" />)).toContain('co\\u{200b}dex');
  });
});

describe('owner-visible label controls', () => {
  it('keeps an ordinary commit subject', async () => {
    expect(await history({ id: '0123456789', subject: 'Fix login', author: 'Ada', atMs: 0 })).toContain('Fix login');
  });

  it('keeps an ordinary commit author', async () => {
    expect(await history({ id: '0123456789', subject: 'Fix', author: 'Ada Lovelace', atMs: 0 })).toContain('Ada Lovelace');
  });

  it('keeps an ordinary attention-strip agent name', () => {
    expect(renderToStaticMarkup(<AttentionStrip agents={[agent('codex')]} />)).toContain('codex');
  });

  it('keeps an ordinary machine name and identity', () => {
    const html = renderToStaticMarkup(<MachineName m={machine('laptop')} />);
    expect(html).toContain('laptop');
    expect(html).toContain('n1');
  });

  it('keeps an ordinary machine-card agent link', () => {
    const html = renderToStaticMarkup(<NetworkView now={NOW} machines={[machine('laptop')]} agents={[agent('codex')]} hubName="hub" />);
    expect(html).toContain('codex');
    expect(html).toContain('#/agent/n1%2Fa1');
  });
});

describe('owner-visible label multi-edge verifier', () => {
  it('covers commit subjects without changing commit identity', async () => {
    for (const [char, escaped] of edges) {
      const out = await history({ id: '0123456789', subject: `su${char}b`, author: 'Ada', atMs: 0 });
      expect(out).toContain(`su${escaped}b`);
      expect(out).toContain('01234567');
    }
  });

  it('covers commit authors without changing commit identity', async () => {
    for (const [char, escaped] of edges) {
      const out = await history({ id: '0123456789', subject: 'Fix', author: `A${char}da`, atMs: 0 });
      expect(out).toContain(`A${escaped}da`);
      expect(out).toContain('01234567');
    }
  });

  it('covers attention-strip names without changing the state label', () => {
    for (const [char, escaped] of edges) {
      const html = renderToStaticMarkup(<AttentionStrip agents={[agent(`co${char}dex`)]} />);
      expect(html).toContain(`co${escaped}dex`);
      expect(html).toContain('Needs you');
    }
  });

  it('covers machine names without changing machine identity', () => {
    for (const [char, escaped] of edges) {
      const html = renderToStaticMarkup(<MachineName m={machine(`ma${char}chine`)} />);
      expect(html).toContain(`ma${escaped}chine`);
      expect(html).toContain('n1');
    }
  });

  it('covers machine-card agent names without changing the agent link', () => {
    for (const [char, escaped] of edges) {
      const html = renderToStaticMarkup(<NetworkView now={NOW} machines={[machine('laptop')]} agents={[agent(`co${char}dex`)]} hubName="hub" />);
      expect(html).toContain(`co${escaped}dex`);
      expect(html).toContain('#/agent/n1%2Fa1');
    }
  });
});
