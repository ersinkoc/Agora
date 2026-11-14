import { expect, it } from 'vitest';
import { directoryMessage, resumeReasonKey, signDirectory } from './directory';
import { thisDevice } from './approval';
import { applyEvent, emptyView, fromSnapshot, tiles } from './view';
import { agora } from '../gen/agora.js';

// ADR-0060.
it('signs a resume for exactly that agent and that choice', async () => {
  const device = await thisDevice({ load: async () => undefined, save: async () => {} });
  const a = await signDirectory(device, 'node', 'resume', 'agent-3', '', '', false);
  const verify = (id: string, fresh: boolean) => crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.publicKey, a.signature as BufferSource,
    directoryMessage('node', 'resume', id, '', '', fresh, false, a.nonce, a.expiresAtMs) as BufferSource);
  expect(await verify('agent-3', false)).toBe(true);
  expect(await verify('agent-4', false)).toBe(false);
  expect(await verify('agent-3', true)).toBe(false);
});

it('keeps an interrupted agent closed under its machine until it starts again, and lists one never seen', () => {
  const v = emptyView();
  applyEvent(v, 1, { producerId: 'n', body: 'nodeOnline', nodeOnline: { nodeId: 'n', hostname: 'Laptop' } });
  applyEvent(v, 2, { producerId: 'n', body: 'agentStarted', agentStarted: { agentId: 'agent-1', nodeId: 'n', projectId: 'p', kind: 'claude-code', worktreeId: '' } });
  applyEvent(v, 3, { producerId: 'n', body: 'nodeOnline', nodeOnline: { nodeId: 'n', hostname: 'Laptop' } });
  applyEvent(v, 4, { producerId: 'n', body: 'agentInterrupted', agentInterrupted: { agentId: 'agent-1', kind: 'claude-code', projectId: 'p', folder: '/work/a', resumable: true, interruptedAtMs: 99 } });
  applyEvent(v, 5, { producerId: 'n', body: 'agentInterrupted', agentInterrupted: { agentId: 'agent-2', kind: 'codex', projectId: 'p', folder: '/work/b', resumable: false, interruptedAtMs: 99 } });
  const [a, b] = tiles(v);
  expect(a).toMatchObject({ id: 'n/agent-1', closed: true, state: 'finished', interrupted: { resumable: true }, endedMs: 99 });
  expect(b).toMatchObject({ id: 'n/agent-2', closed: true, workspace: '/work/b', machine: 'Laptop', interrupted: { resumable: false } });
  applyEvent(v, 6, { producerId: 'n', body: 'agentStarted', agentStarted: { agentId: 'agent-1', nodeId: 'n', projectId: 'p', kind: 'claude-code', worktreeId: '' } });
  expect(tiles(v)[0]).toMatchObject({ closed: false });
  expect(tiles(v)[0]?.interrupted).toBeUndefined();
  const snap = fromSnapshot(agora.v1.Snapshot.fromObject({ pos: 7, nodes: [{ nodeId: 'n', online: true }], agents: [{ agentId: 'n/agent-2', nodeId: 'n', kind: 'codex', running: false, interrupted: true, resumable: false, interruptedAtMs: 99, folder: '/work/b' }] }));
  expect(tiles(snap)[0]).toMatchObject({ closed: true, workspace: '/work/b', interrupted: { resumable: false } });
});

it('marks an interrupted agent whose process outlived the restart until the node says it is stopped', () => {
  const v = emptyView();
  applyEvent(v, 1, { producerId: 'n', body: 'agentInterrupted', agentInterrupted: { agentId: 'agent-1', kind: 'codex', folder: '/w', resumable: true, stillRunning: true } });
  expect(tiles(v)[0]?.interrupted).toEqual({ resumable: true, stillRunning: true });
  applyEvent(v, 2, { producerId: 'n', body: 'agentInterrupted', agentInterrupted: { agentId: 'agent-1', kind: 'codex', folder: '/w', resumable: true } });
  expect(tiles(v)[0]?.interrupted).toEqual({ resumable: true, stillRunning: false });
});

it('names why a resume did not happen in words of its own', () => {
  expect(resumeReasonKey('agent.folder_changed')).toBe('resume-failed-folder');
  expect(resumeReasonKey('agent.parent_not_running')).toBe('resume-failed-parent');
  expect(resumeReasonKey('agent.still_running')).toBe('resume-failed-still-running');
  expect(resumeReasonKey('something.new')).toBe('resume-failed');
});
