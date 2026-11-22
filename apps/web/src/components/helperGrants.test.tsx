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

  it('reads the list again after each of two revokes made in the same view', async () => {
    let server: HelperGrant[] = [
      { id: 'g1', kind: 'codex', folder: '/one', remaining: 1 },
      { id: 'g2', kind: 'codex', folder: '/two', remaining: 1 },
    ];
    const list = () => Promise.resolve(server.map((g) => ({ ...g })));
    const done = new Map<string, () => void>();
    const revoke = (id: string) => new Promise<void>((r) => { done.set(id, () => { server = server.filter((g) => g.id !== id); r(); }); });
    await act(async () => { root.render(<HelperGrants list={list} revoke={revoke} refresh={0} />); });
    const buttons = () => Array.from(host.querySelectorAll('button'));
    expect(buttons()).toHaveLength(2);
    await act(async () => { buttons()[0]!.click(); buttons()[1]!.click(); });
    await act(async () => { done.get('g1')!(); });
    await act(async () => { done.get('g2')!(); });
    expect(host.textContent).not.toContain('/one');
    expect(host.textContent).not.toContain('/two');
  });

  it('negative control: a revoke that ends after the view moved to another agent does nothing there', async () => {
    let finish!: () => void;
    let reads = 0;
    const listA = () => Promise.resolve([{ id: 'a1', kind: 'codex', folder: '/a', remaining: 1 }]);
    const listB = () => { reads++; return Promise.resolve([{ id: 'b1', kind: 'codex', folder: '/b', remaining: 1 }]); };
    await act(async () => { root.render(<HelperGrants list={listA} revoke={() => new Promise<void>((r) => { finish = r; })} refresh={0} />); });
    await act(async () => { host.querySelector('button')!.click(); });
    await act(async () => { root.render(<HelperGrants list={listB} revoke={async () => {}} refresh={0} />); });
    const before = reads;
    await act(async () => { finish(); });
    expect(reads).toBe(before);
    expect(host.textContent).toContain('/b');
  });
});
