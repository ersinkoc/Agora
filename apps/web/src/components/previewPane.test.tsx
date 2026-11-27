// @vitest-environment jsdom
// ADR-0088 step 4: the Preview pane enters only the forwards this browser opened, in a sandboxed frame on the forward's own origin;
// opens a port with what the form says; shows a machine's suggestion as a card (never an open by itself) and a refusal in words.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { PreviewActions, PreviewState } from '../layout/panels';
import type { Forward } from '../net/preview';
import { PreviewPane } from './PreviewPane';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const machines = [{ id: 'mac', name: 'mac', os: 'macos', online: true, agentsRunning: 1, lastSeenMs: 1 }];
const fwd = (over: Partial<Forward> = {}): Forward => ({ node: 'mac', id: 'fwd-1', port: 5173, openedMs: 1, endsMs: Date.now() + 3_600_000, creator: 'web-me', ...over });
const state = (over: Partial<PreviewState> = {}): PreviewState => ({ forwards: [], suggestions: [], device: 'web-me', hostname: 'hub.example', ...over });
function actions(over: Partial<PreviewActions> = {}): PreviewActions {
  return {
    open: vi.fn(async (node: string, port: number) => ({ ok: true as const, forward: fwd({ node, port }) })),
    close: vi.fn(async () => undefined),
    enter: vi.fn(async () => ({ ok: true as const, port: 7451, ticket: 'T' })),
    ...over,
  };
}

let root: Root;
let host: HTMLDivElement;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
const render = async (s: PreviewState | undefined, a: PreviewActions | undefined) => {
  await act(async () => { root.render(<PreviewPane state={s} actions={a} machines={machines} fleet={[]} />); });
};
const button = (text: string) => [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === text || b.getAttribute('aria-label') === text);

describe('the Preview pane', () => {
  it('says so where previews are not available', async () => {
    await render(undefined, undefined);
    expect(host.textContent).toContain('Previews are not available from this window.');
  });

  it('opens the port the form names, for the hours chosen', async () => {
    const a = actions();
    await render(state(), a);
    const port = host.querySelector('input') as HTMLInputElement;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(port, '5173');
      port.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => { button('Open')?.click(); });
    expect(a.open).toHaveBeenCalledWith('mac', 5173, 8 * 3600);
  });

  it("enters a forward this browser opened in a sandboxed frame on the forward's origin", async () => {
    const a = actions();
    await render(state({ forwards: [fwd()] }), a);
    expect(a.enter).toHaveBeenCalledTimes(1);
    const frame = host.querySelector('iframe')!;
    expect(frame.getAttribute('src')).toBe('https://hub.example:7451/.agora-preview/enter?t=T');
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts allow-forms allow-same-origin allow-popups');
    expect(frame.getAttribute('referrerpolicy')).toBe('no-referrer');
    expect(host.textContent).toContain('mac · port 5173');
    await act(async () => { button('Close')?.click(); });
    expect(a.close).toHaveBeenCalledWith(expect.objectContaining({ id: 'fwd-1' }));
  });

  it("lists another device's forward without entering it", async () => {
    const a = actions();
    await render(state({ forwards: [fwd({ creator: 'web-other' })] }), a);
    expect(a.enter).not.toHaveBeenCalled();
    expect(host.querySelector('iframe')).toBeNull();
    expect(host.textContent).toContain('opened on another device');
  });

  it("shows a machine's suggestion as a card that opens only when pressed", async () => {
    const a = actions();
    await render(state({ suggestions: [{ node: 'mac', agentId: '', port: 3000, note: 'vite', atMs: 1 }] }), a);
    expect(host.textContent).toContain('Something listens on port 3000 on mac');
    expect(a.open).not.toHaveBeenCalled();
    const open = [...host.querySelectorAll('.preview-suggestion button')][0] as HTMLButtonElement;
    await act(async () => { open.click(); });
    expect(a.open).toHaveBeenCalledWith('mac', 3000, 8 * 3600);
  });

  it('says in words why it could not enter', async () => {
    await render(state({ forwards: [fwd()] }), actions({ enter: vi.fn(async () => ({ ok: false as const, code: 'preview.unavailable', reason: 'no_origin' })) }));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('through a proxy or another port');
  });
});
