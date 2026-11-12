import { describe, expect, it } from 'vitest';
import { boards, fromSnapshot } from './view';

describe('board identity', () => {
  it('distinguishes equal folder names, while grouping copies of the same project', () => {
    const view = fromSnapshot({
      nodes: [{ nodeId: 'linux', hostname: 'Laptop' }, { nodeId: 'mac', hostname: 'Laptop' }],
      checkouts: [
        { checkoutId: 'linux/a', nodeId: 'linux', projectId: 'project-a', path: '/one/shared' },
        { checkoutId: 'mac/a', nodeId: 'mac', projectId: 'project-a', path: '/copy/shared' },
        { checkoutId: 'linux/b', nodeId: 'linux', projectId: 'project-b', path: '/two/shared' },
      ],
      tasks: [
        { taskId: 't-1', projectId: 'project-a', title: 'First project' },
        { taskId: 't-1', projectId: 'project-b', title: 'Second project' },
      ],
    });
    const result = boards(view);
    expect(result.map(board => board.project)).toEqual(['shared', 'shared']);
    expect(result.map(board => board.projectId)).toEqual(['project-a', 'project-b']);
    expect(result[0]?.folders).toEqual([
      { nodeId: 'linux', machine: 'Laptop', path: '/one/shared' },
      { nodeId: 'mac', machine: 'Laptop', path: '/copy/shared' },
    ]);
    expect(result[0]?.tasks.map(task => task.title)).toEqual(['First project']);
    expect(result[1]?.tasks.map(task => task.title)).toEqual(['Second project']);
  });

  it('keeps selected-folder leases on their project, deduplicating its checkout context', () => {
    const view = fromSnapshot({
      nodes: [{ nodeId: 'n', hostname: 'Machine' }],
      checkouts: [{ checkoutId: 'n/c', nodeId: 'n', projectId: 'git-project', path: '/repo' }],
      workspaces: [
        { workspaceId: 'n/w', nodeId: 'n', projectId: 'git-project', path: '/repo/src', gitRoot: '/repo' },
        { workspaceId: 'n/folder', nodeId: 'n', projectId: 'folder-project', path: '/notes' },
      ],
      agentWorkspaces: [{ agentId: 'n/a', workspaceId: 'n/w' }, { agentId: 'n/b', workspaceId: 'n/folder' }],
      agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'shell' }, { agentId: 'n/b', nodeId: 'n', kind: 'shell' }],
      leases: [
        { agentId: 'n/a', path: 'src/lib.rs', expiresAtMs: 9000 },
        { agentId: 'n/b', path: 'todo.txt', expiresAtMs: 9000 },
      ],
    });
    const result = boards(view);
    expect(result).toHaveLength(2);
    expect(result.find(board => board.projectId === 'git-project')).toMatchObject({
      project: 'repo', folders: [{ nodeId: 'n', machine: 'Machine', path: '/repo' }],
      leases: [{ path: 'src/lib.rs' }],
    });
    expect(result.find(board => board.projectId === 'folder-project')).toMatchObject({
      project: 'notes', folders: [{ nodeId: 'n', machine: 'Machine', path: '/notes' }],
      leases: [{ path: 'todo.txt' }],
    });
  });

  it('retains an unknown project identity until its folder facts arrive', () => {
    const view = fromSnapshot({ tasks: [{ taskId: 't-1', projectId: 'unseen-project', title: 'Pending context' }] });
    expect(boards(view)[0]).toMatchObject({ projectId: 'unseen-project', project: 'unseen-project', folders: [] });
    expect(view.checkouts.size).toBe(0);
    expect(view.workspaces.size).toBe(0);
  });

  it('keeps the checkout name when an isolated workspace sorts before it', () => {
    const view = fromSnapshot({
      checkouts: [{ checkoutId: 'z/c', nodeId: 'z', projectId: 'p', path: '/repos/shop' }],
      workspaces: [{ workspaceId: 'a/w', nodeId: 'a', projectId: 'p', path: '/agents/session-1', gitRoot: '/agents/session-1' }],
      tasks: [{ taskId: 't-1', projectId: 'p', title: 'Keep the repository name' }],
    });
    expect(boards(view)[0]?.project).toBe('shop');
  });

  it('groups 500 selected-directory sessions into 50 boards with two checkout contexts each', () => {
    const workspaces = Array.from({ length: 500 }, (_, i) => ({
      workspaceId: `w${i}`, nodeId: `node-${i % 2}`, projectId: `p${Math.floor(i / 10)}`,
      path: `/repos/p${Math.floor(i / 10)}/session-${i}`, gitRoot: `/repos/p${Math.floor(i / 10)}`,
    }));
    const view = fromSnapshot({
      workspaces,
      agents: workspaces.map((w, i) => ({ agentId: `a${i}`, nodeId: w.nodeId, kind: 'shell' })),
      agentWorkspaces: workspaces.map((w, i) => ({ agentId: `a${i}`, workspaceId: w.workspaceId })),
      leases: workspaces.map((_, i) => ({ agentId: `a${i}`, path: `file-${i}`, expiresAtMs: 9000 })),
    });
    for (let i = 0; i < 10; i++) boards(view);
    const samples = Array.from({ length: 50 }, () => {
      const start = performance.now();
      const result = boards(view);
      const elapsed = performance.now() - start;
      expect(result).toHaveLength(50);
      expect(result.every(board => board.folders.length === 2 && board.leases.length === 10)).toBe(true);
      return elapsed;
    }).sort((a, b) => a - b);
    console.info(`Board projection: 500 agents, 50 projects, 50 samples, p95 ${samples[47]?.toFixed(2)} ms (projection only).`);
  });

  it('does not list an isolated working copy as a folder of its board', () => {
    const view = fromSnapshot({
      nodes: [{ nodeId: 'n', hostname: 'Machine' }],
      checkouts: [{ checkoutId: 'n/c', nodeId: 'n', projectId: 'p', path: '/repo' }],
      tasks: [{ taskId: 't', projectId: 'p', title: 'T' }],
      workspaces: [{ workspaceId: 'n/wt', nodeId: 'n', projectId: 'p', path: '/wt/copy', gitRoot: '/wt/copy', managedWorktree: true }],
    });
    expect(boards(view)[0]?.folders.map(f => f.path)).toEqual(['/repo']);
  });
});

import { bytesLabel } from './view';
describe('bytesLabel', () => {
  it('writes memory the way people read it', () => {
    expect(bytesLabel(0)).toBe('');
    expect(bytesLabel(512)).toBe('512 B');
    expect(bytesLabel(1536 * 1024 * 1024)).toBe('1.5 GB');
    expect(bytesLabel(20 * 1024 * 1024)).toBe('20 MB');
  });
});

import { tiles, applyEvent } from './view';
describe('closed agents', () => {
  it('are told apart from running ones, with how they ended', () => {
    const view = fromSnapshot({ nodes: [{ nodeId: 'n', hostname: 'M' }], agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'claude-code', running: true }] });
    applyEvent(view, 5, { producerId: 'n', body: 'agentExited', agentExited: { agentId: 'a', exitCode: 2 } } as never);
    const t = tiles(view)[0];
    expect(t?.closed).toBe(true);
    expect(t?.exitCode).toBe(2);
    expect(t?.state).toBe('finished');
  });
});

describe('forgotten agents', () => {
  it('leave the view when the hub says it forgot them', () => {
    const view = fromSnapshot({ nodes: [{ nodeId: 'n', hostname: 'M' }], agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'shell', running: false }] });
    expect(tiles(view)).toHaveLength(1);
    applyEvent(view, 6, { producerId: 'hub', body: 'agentForgotten', agentForgotten: { agentId: 'n/a' } } as never);
    expect(tiles(view)).toHaveLength(0);
  });
});
