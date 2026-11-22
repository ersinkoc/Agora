// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FilesView } from './FilesView';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  host.remove();
});

describe('FilesView', () => {
  it('cancels pending list retries when unmounted', async () => {
    vi.useFakeTimers();
    try {
      let calls = 0;
      const files = {
        list: () => { calls++; return Promise.reject(new Error('not ready')); },
        read: () => Promise.resolve(new Uint8Array()),
      };
      await act(async () => { root.render(<FilesView files={files} />); });
      await act(async () => { await Promise.resolve(); });
      const beforeUnmount = calls;
      expect(beforeUnmount).toBeGreaterThanOrEqual(1);

      await act(async () => { root.unmount(); });
      await act(async () => { vi.advanceTimersByTime(5000); await Promise.resolve(); });
      expect(calls).toBe(beforeUnmount);
    } finally {
      vi.useRealTimers();
    }
  });
});
