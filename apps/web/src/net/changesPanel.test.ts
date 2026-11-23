import { expect, it } from 'vitest';
import { changedFolders } from './paneTools';
import { applyEvent, emptyView, tiles } from './view';

// #1017: two agents started one after the other in one folder (no separate workspace). The machine registers the folder again for the
// second agent, with no count (it is counted by its Git watcher, which reported it once already); the right panel's Changes must still
// list the folder with its changed files, as the review of either agent does.
it('lists a shared folder with its changes after a second agent registers it again', () => {
  const v = emptyView();
  let pos = 0;
  const ev = (body: string, value: object) => applyEvent(v, ++pos, { producerId: 'n', body, [body]: value } as Parameters<typeof applyEvent>[2]);
  ev('nodeOnline', { nodeId: 'n', hostname: 'blackview' });
  const folder = { workspaceId: 'directory:/work/proj', nodeId: 'n', projectId: 'id:proj', path: '/work/proj', gitRoot: '/work/proj', branch: 'main', filesChanged: 0 };
  const start = (agentId: string) => {
    ev('workspaceRegistered', folder);
    ev('agentWorkspaceBound', { agentId, workspaceId: folder.workspaceId });
    ev('agentStarted', { agentId, nodeId: 'n', projectId: 'id:proj', kind: 'shell' });
  };
  start('agent-1');
  ev('workspaceStatusChanged', { workspaceId: folder.workspaceId, filesChanged: 3, insertions: 9, deletions: 11 });
  start('agent-2');

  const fleet = tiles(v);
  expect(fleet.map((a) => a.filesChanged)).toEqual([3, 3]);
  const changed = changedFolders(fleet);
  expect(changed).toHaveLength(1);
  expect(changed[0]?.shared).toBe(true);
  expect(changed[0]?.workspace).toBe('/work/proj');
  expect(changed[0]?.filesChanged).toBe(3);
});

it('lists a folder once however many agents share it, and only the chosen project', () => {
  const v = emptyView();
  let pos = 0;
  const ev = (body: string, value: object) => applyEvent(v, ++pos, { producerId: 'n', body, [body]: value } as Parameters<typeof applyEvent>[2]);
  ev('nodeOnline', { nodeId: 'n', hostname: 'blackview' });
  for (const [ws, project, agents] of [['directory:/a', 'id:a', ['agent-1', 'agent-2']], ['directory:/b', 'id:b', ['agent-3']]] as const) {
    for (const agentId of agents) {
      ev('workspaceRegistered', { workspaceId: ws, nodeId: 'n', projectId: project, path: ws.slice(10), gitRoot: ws.slice(10), filesChanged: 0 });
      ev('agentWorkspaceBound', { agentId, workspaceId: ws });
      ev('agentStarted', { agentId, nodeId: 'n', projectId: project, kind: 'shell' });
    }
    ev('workspaceStatusChanged', { workspaceId: ws, filesChanged: project === 'id:a' ? 1 : 4 });
  }
  expect(changedFolders(tiles(v)).map((a) => [a.workspace, a.filesChanged])).toEqual([['/b', 4], ['/a', 1]]);
  expect(changedFolders(tiles(v), 'id:a').map((a) => a.workspace)).toEqual(['/a']);
});
