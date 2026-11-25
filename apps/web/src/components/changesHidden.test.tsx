// @vitest-environment jsdom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it } from 'vitest';
import { ChangesView } from './ChangesView';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
HTMLElement.prototype.scrollIntoView = () => {};

describe('Changes path text', () => {
  it('makes folder, current, and renamed paths visible', async () => {
    const path = 'src/Grü\u3164ße\u2800.ts';
    const old = 'old/ça\u202elış.ts';
    const diff = `diff --git a/${old} b/${path}\nsimilarity index 90%\nrename from ${old}\nrename to ${path}\n--- a/${old}\n+++ b/${path}\n@@ -1 +1 @@\n-a\n+b\n`;
    const host = document.createElement('div'); document.body.append(host); const root = createRoot(host);
    await act(async () => { root.render(<ChangesView load={() => Promise.resolve(diff)} />); await Promise.resolve(); });
    const shown = host.textContent ?? '';
    expect([...shown].some((c) => ['\u3164', '\u2800', '\u202e'].includes(c))).toBe(false);
    expect(shown).toContain('Grü\\u{3164}ße\\u{2800}.ts');
    expect(shown).toContain('ça\\u{202e}lış.ts');
    act(() => root.unmount()); host.remove();
  });
  it('verifies multilingual names in rows, headings, and tooltips', async () => {
    const path = 'src/漢\u202e字\u3164.rs';
    const diff = `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-a\n+b\n`;
    const host = document.createElement('div'); document.body.append(host); const root = createRoot(host);
    await act(async () => { root.render(<ChangesView load={() => Promise.resolve(diff)} />); await Promise.resolve(); });
    expect(host.textContent).toContain('漢\\u{202e}字\\u{3164}.rs');
    expect(host.querySelector('[role=option]')?.getAttribute('title')).toBe('src/漢\\u{202e}字\\u{3164}.rs');
    act(() => root.unmount()); host.remove();
  });
});
