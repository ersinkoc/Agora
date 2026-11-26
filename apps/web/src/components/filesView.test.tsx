// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { t } from '../i18n';
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
  it('makes changed-only file paths visible', async () => {
    const raw = 'src/Grü\u3164ße\u2800.ts';
    const diff = `diff --git a/${raw} b/${raw}\n--- a/${raw}\n+++ b/${raw}\n@@ -1 +1 @@\n-a\n+b\n`;
    const files = { list: () => Promise.resolve([]), read: () => Promise.resolve(new Uint8Array()) };
    await act(async () => { root.render(<FilesView files={files} load={() => Promise.resolve(diff)} changed={1} />); await Promise.resolve(); });
    const changed = [...host.querySelectorAll('button')].find((b) => b.textContent?.includes('changed')) as HTMLButtonElement;
    await act(async () => { changed.click(); });
    const shown = host.textContent ?? '';
    expect([...shown].some((c) => ['\u3164', '\u2800'].includes(c))).toBe(false);
    expect(shown).toContain('src/Grü\\u{3164}ße\\u{2800}.ts');
  });
  it('verifies changed-only display while preserving the raw path identity', async () => {
    const raw = 'src/漢\u202e字\u3164.rs';
    const diff = `diff --git a/${raw} b/${raw}\n--- a/${raw}\n+++ b/${raw}\n@@ -1 +1 @@\n-a\n+b\n`;
    const files = { list: () => Promise.resolve([]), read: () => Promise.resolve(new Uint8Array()) };
    await act(async () => { root.render(<FilesView files={files} load={() => Promise.resolve(diff)} changed={1} />); await Promise.resolve(); });
    const changed = [...host.querySelectorAll('button')].find((b) => b.textContent?.includes('changed')) as HTMLButtonElement;
    await act(async () => { changed.click(); });
    const row = host.querySelector('[role=treeitem]') as HTMLElement;
    expect(row.textContent).toContain('src/漢\\u{202e}字\\u{3164}.rs');
    expect(row.dataset.path).toBe(raw);
  });

  it('makes open-file tabs, breadcrumbs, and region labels visible', async () => {
    const raw = 'Grü\u3164ße\u2800.ts';
    const files = { list: () => Promise.resolve([{ name: raw, isDir: false, size: 1 }]), read: () => Promise.resolve(new TextEncoder().encode('x')) };
    await act(async () => { root.render(<FilesView files={files} />); });
    await act(async () => { host.querySelector<HTMLElement>(`[data-path="${raw}"]`)!.click(); await Promise.resolve(); });
    const shown = host.textContent ?? '';
    expect([...shown].some((c) => ['\u3164', '\u2800'].includes(c))).toBe(false);
    expect(shown).toContain('Grü\\u{3164}ße\\u{2800}.ts');
    expect(host.querySelector('[role=region]')?.getAttribute('aria-label')).toBe('Grü\\u{3164}ße\\u{2800}.ts');
  });
  it('verifies open-file chrome while reading the raw path', async () => {
    const raw = '漢\u202e字\u3164.rs';
    const read = vi.fn(() => Promise.resolve(new TextEncoder().encode('x')));
    const files = { list: () => Promise.resolve([{ name: raw, isDir: false, size: 1 }]), read };
    await act(async () => { root.render(<FilesView files={files} />); });
    await act(async () => { host.querySelector<HTMLElement>(`[data-path="${raw}"]`)!.click(); await Promise.resolve(); });
    expect(host.querySelector('[role=tab]')?.textContent).toContain('漢\\u{202e}字\\u{3164}.rs');
    expect(host.querySelector('[role=region]')?.getAttribute('aria-label')).toBe('漢\\u{202e}字\\u{3164}.rs');
    expect(read).toHaveBeenCalledWith(raw);
  });

  it('makes listed file names visible', async () => {
    const files = { list: () => Promise.resolve([{ name: 'Grü\u3164ße\u2800.ts', isDir: false, size: 1 }]), read: () => Promise.resolve(new Uint8Array()) };
    await act(async () => { root.render(<FilesView files={files} />); });
    const shown = host.textContent ?? '';
    expect([...shown].some((c) => ['\u3164', '\u2800'].includes(c))).toBe(false);
    expect(shown).toContain('Grü\\u{3164}ße\\u{2800}.ts');
  });
  it('verifies multilingual display while preserving the raw file path', async () => {
    const raw = '漢\u202e字\u3164.rs';
    const files = { list: () => Promise.resolve([{ name: raw, isDir: false, size: 1 }]), read: () => Promise.resolve(new Uint8Array()) };
    await act(async () => { root.render(<FilesView files={files} />); });
    expect(host.textContent).toContain('漢\\u{202e}字\\u{3164}.rs');
    expect(host.querySelector('[role=treeitem]')?.getAttribute('data-path')).toBe(raw);
  });

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

  it('shows a file whose read was still on its way when the tree was refreshed', async () => {
    let finish!: (b: Uint8Array) => void;
    const files = {
      list: () => Promise.resolve([{ name: 'a.txt', isDir: false, size: 5 }]),
      read: () => new Promise<Uint8Array>((r) => { finish = r; }),
    };
    await act(async () => { root.render(<FilesView files={files} />); });
    const row = host.querySelector<HTMLElement>('[data-path="a.txt"]');
    expect(row).not.toBeNull();
    await act(async () => { row!.click(); });
    const refresh = host.querySelector<HTMLElement>(`button[aria-label="${t('files-refresh')}"]`);
    expect(refresh).not.toBeNull();
    await act(async () => { refresh!.click(); });
    await act(async () => { finish(new TextEncoder().encode('hello')); await Promise.resolve(); });
    expect(host.textContent).toContain('hello');
    expect(host.querySelector('[aria-busy="true"]')).toBeNull();
    await act(async () => { root.unmount(); });
  });

  it('negative control: a read for workspace files that were replaced is not shown', async () => {
    let finish!: (b: Uint8Array) => void;
    const old = {
      list: () => Promise.resolve([{ name: 'a.txt', isDir: false, size: 5 }]),
      read: () => new Promise<Uint8Array>((r) => { finish = r; }),
    };
    const next = { list: old.list, read: () => Promise.resolve(new TextEncoder().encode('fresh')) };
    await act(async () => { root.render(<FilesView files={old} />); });
    await act(async () => { host.querySelector<HTMLElement>('[data-path="a.txt"]')!.click(); });
    await act(async () => { root.render(<FilesView files={next} />); });
    await act(async () => { finish(new TextEncoder().encode('stale')); await Promise.resolve(); });
    expect(host.textContent).not.toContain('stale');
    await act(async () => { root.unmount(); });
  });
});
