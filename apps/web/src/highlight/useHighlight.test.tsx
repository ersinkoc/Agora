// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let failFirst = true;
vi.mock('./hl', () => {
  if (failFirst) {
    failFirst = false;
    return Promise.reject(new Error('transient chunk load failure'));
  }
  return {
    highlight: (code: string) => `<span class="hl">${code}</span>`,
    highlightLine: (line: string) => `<span class="hl">${line}</span>`,
  };
});

import { useHighlightedFile } from './useHighlight';

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  host.remove();
});

function Probe({ path, lines }: { path: string; lines: string[] }) {
  const html = useHighlightedFile(path, lines);
  return <div data-hl={html ? 'yes' : 'no'}>{html?.join('\n') ?? 'plain'}</div>;
}

describe('useHighlight', () => {
  it('retries loading the highlighter chunk after a transient import failure', async () => {
    vi.useFakeTimers();
    try {
      await act(async () => { root.render(<Probe path="a.ts" lines={['const x = 1;']} />); });
      await act(async () => { vi.runAllTimers(); await Promise.resolve(); await Promise.resolve(); });
      expect(host.querySelector('[data-hl]')?.getAttribute('data-hl')).toBe('no');

      await act(async () => { root.render(<Probe path="b.ts" lines={['const y = 2;']} />); });
      await act(async () => { vi.runAllTimers(); await Promise.resolve(); await Promise.resolve(); });
      expect(host.querySelector('[data-hl]')?.getAttribute('data-hl')).toBe('yes');
    } finally {
      vi.useRealTimers();
    }
  });
});
