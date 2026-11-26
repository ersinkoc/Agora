// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { DeviceRequest } from './DeviceRequest';
import { DiffDialog } from './DiffDialog';
import { FilesDialog } from './FilesDialog';
import { HistoryDialog } from './HistoryDialog';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); document.body.replaceChildren(); });

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id: 'n1/a1', name: 'clau\u3164de\u202e', accent: '#fff', machine: 'lap\u034ftop', workspace: 'wo\u2066rk', project: 'lo\u200bgin', branch: 'fi\ue000x', state: 'working', preview: '', ...over,
});
const shown = ['clau\\u{3164}de\\u{202e}', 'lap\\u{34f}top', 'wo\\u{2066}rk', 'lo\\u{200b}gin', 'fi\\u{e000}x'];
const dialogText = () => document.querySelector('[role=dialog]')?.textContent ?? '';

describe('operational dialog labels', () => {
  it('makes device request and machine names visible', () => {
    act(() => root.render(<DeviceRequest inline request={{ id: 'web-1', name: 'Fire\u3164fox\u202e', fingerprint: 'abcd' }} machines={[{ id: 'n1', name: 'lap\u034ftop\ue000' }]} onApprove={async () => true} onLater={() => {}} />));
    expect(dialogText()).toContain('Fire\\u{3164}fox\\u{202e}');
    expect(dialogText()).toContain('lap\\u{34f}top\\u{e000}');
  });

  it('verifies device labels across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u200d', '\\u{200d}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const machines = [{ id: `raw${char}id`, name: `ma${char}chine` }];
      act(() => root.render(<DeviceRequest inline request={{ id: 'web-1', name: `de${char}vice`, fingerprint: 'abcd' }} machines={machines} onApprove={async () => true} onLater={() => {}} />));
      expect(dialogText()).toContain(`de${escaped}vice`);
      expect(dialogText()).toContain(`ma${escaped}chine`);
      expect(machines[0]?.id).toBe(`raw${char}id`);
    }
  });

  it('makes Files dialog agent and location labels visible', () => {
    act(() => root.render(<FilesDialog agent={agent()} files={{ list: () => new Promise(() => {}), read: () => new Promise(() => {}) }} onClose={() => {}} />));
    for (const text of [shown[0], shown[1], shown[3], shown[4]]) expect(dialogText()).toContain(text);
  });

  it('verifies Files labels across isolate, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u2066', '\\u{2066}'], ['\u0001', '\\u{1}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      act(() => root.render(<FilesDialog agent={agent({ name: `co${char}dex`, machine: `ma${char}chine`, project: `pro${char}ject`, branch: `br${char}anch` })} files={{ list: () => new Promise(() => {}), read: () => new Promise(() => {}) }} onClose={() => {}} />));
      for (const [start, end] of [['co', 'dex'], ['ma', 'chine'], ['pro', 'ject'], ['br', 'anch']] as const) expect(dialogText()).toContain(`${start}${escaped}${end}`);
    }
  });

  it('makes History dialog agent and location labels visible', () => {
    act(() => root.render(<HistoryDialog agent={agent()} load={() => new Promise(() => {})} onClose={() => {}} />));
    for (const text of shown.slice(0, 3)) expect(dialogText()).toContain(text);
  });

  it('verifies History labels across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u200b', '\\u{200b}'], ['\u0007', '\\u{7}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      act(() => root.render(<HistoryDialog agent={agent({ name: `co${char}dex`, machine: `ma${char}chine`, workspace: `wo${char}rk` })} load={() => new Promise(() => {})} onClose={() => {}} />));
      for (const [start, end] of [['co', 'dex'], ['ma', 'chine'], ['wo', 'rk']] as const) expect(dialogText()).toContain(`${start}${escaped}${end}`);
    }
  });

  it('makes Changes dialog agent and location labels visible', () => {
    act(() => root.render(<DiffDialog agent={agent()} load={() => new Promise(() => {})} onClose={() => {}} />));
    for (const text of [shown[0], shown[1], shown[3], shown[4]]) expect(dialogText()).toContain(text);
  });

  it('verifies Changes labels across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u2069', '\\u{2069}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      act(() => root.render(<DiffDialog agent={agent({ name: `co${char}dex`, machine: `ma${char}chine`, project: `pro${char}ject`, branch: `br${char}anch` })} load={() => new Promise(() => {})} onClose={() => {}} />));
      for (const [start, end] of [['co', 'dex'], ['ma', 'chine'], ['pro', 'ject'], ['br', 'anch']] as const) expect(dialogText()).toContain(`${start}${escaped}${end}`);
    }
  });
});
