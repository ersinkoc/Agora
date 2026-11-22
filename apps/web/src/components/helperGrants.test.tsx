// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { HelperGrants, type HelperGrant } from './HelperGrants';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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

describe('HelperGrants', () => {
  it('ignores stale list() resolutions and rejections after props change', async () => {
    let resolveA!: (g: HelperGrant[]) => void;
    let rejectA!: (err: Error) => void;
    const listA1 = () => new Promise<HelperGrant[]>((r) => { resolveA = r; });
    const listA2 = () => new Promise<HelperGrant[]>((_, rej) => { rejectA = rej; });
    const listB = () => Promise.resolve([{ id: 'b1', kind: 'codex', folder: '/b-new', remaining: 1 }]);

    await act(async () => {
      root.render(<HelperGrants list={listA1} revoke={async () => {}} refresh={0} />);
    });
    await act(async () => {
      root.render(<HelperGrants list={listA2} revoke={async () => {}} refresh={1} />);
    });
    await act(async () => {
      root.render(<HelperGrants list={listB} revoke={async () => {}} refresh={2} />);
    });
    expect(host.textContent).toContain('/b-new');

    await act(async () => {
      resolveA([{ id: 'a1', kind: 'claude', folder: '/a-stale', remaining: 2 }]);
    });
    await act(async () => {
      rejectA(new Error('stale error'));
    });

    expect(host.textContent).toContain('/b-new');
    expect(host.textContent).not.toContain('/a-stale');
    expect(host.querySelector('[role="status"]')).toBeNull();
  });
});
