// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { t } from '../i18n';
import type { DirectoryListing } from '../net/directory';
import type { MachineModel } from '../net/view';
import { StartAgent } from './StartAgent';

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

describe('StartAgent', () => {
  it('ignores stale directory listings from a previous machine after switching', async () => {
    const machines: MachineModel[] = [
      { id: 'n1', name: 'Node 1', online: true, os: 'linux', binary: '', agentsRunning: 0, terminalsOpen: 0, memoryBytes: 0, lastSeenMs: 0 },
      { id: 'n2', name: 'Node 2', online: true, os: 'linux', binary: '', agentsRunning: 0, terminalsOpen: 0, memoryBytes: 0, lastSeenMs: 0 },
    ];
    let resolveN1!: (v: DirectoryListing) => void;
    const list = (node: string) =>
      node === 'n1'
        ? new Promise<DirectoryListing>((r) => { resolveN1 = r; })
        : Promise.resolve({ path: '/n2-home', parent: '/', entries: [{ name: 'n2-folder', isGit: false }], truncated: false });

    await act(async () => {
      root.render(
        <StartAgent
          projects={[]}
          machines={machines}
          initialNode="n1"
          loadKinds={() => Promise.resolve([])}
          inspect={() => Promise.reject(new Error('unused'))}
          list={list}
          onStart={() => Promise.resolve(undefined)}
          onClose={() => {}}
        />,
      );
    });

    const browseBtn = Array.from(document.body.querySelectorAll('button')).find((b) => b.textContent?.includes(t('start-browse')));
    await act(async () => { browseBtn!.click(); });

    const n2Btn = Array.from(document.body.querySelectorAll('button')).find((b) => b.textContent?.includes('Node 2'));
    await act(async () => { n2Btn!.click(); });

    await act(async () => {
      resolveN1({ path: '/n1-stale-home', parent: '/', entries: [{ name: 'n1-stale-folder', isGit: false }], truncated: false });
    });

    expect(document.body.textContent ?? '').not.toContain('n1-stale-folder');
  });
});
