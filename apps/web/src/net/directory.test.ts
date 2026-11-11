import { expect, it } from 'vitest';
import { directoryMessage, signDirectory } from './directory';
import { thisDevice } from './approval';
import { applyEvent, emptyView, fromSnapshot, tiles, projectChoices } from './view';
import { machineKey } from '../model';
import { agora } from '../gen/agora.js';
it('binds selected directory, node, project, launch mode and identity confirmation in the signature', async () => {
  const device = await thisDevice({ load: async () => undefined, save: async () => {} });
  const a = await signDirectory(device, 'node', 'launch', '/selected', 'codex', 'id:p');
  const verify = (path:string, separate=false, identity=false) => crypto.subtle.verify({name:'ECDSA',hash:'SHA-256'},device.keys.publicKey,a.signature as BufferSource,directoryMessage('node','launch',path,'codex','id:p',separate,identity,a.nonce,a.expiresAtMs) as BufferSource);
  expect(await verify('/selected')).toBe(true);expect(await verify('/elsewhere')).toBe(false);expect(await verify('/selected',true)).toBe(false);expect(await verify('/selected',false,true)).toBe(false);
  expect(new TextDecoder().decode(directoryMessage('n','launch','/p','k','id',false,false,'nonce',5))).toBe('directory-action-v1\0n\0launch\0/p\0k\0id\x000\x000\0nonce\x005');
});
it('keeps workspace binding through reload and identifies shared folders without inventing worktrees', () => {
  const v=emptyView();applyEvent(v,1,{producerId:'n',body:'nodeOnline',nodeOnline:{nodeId:'n',hostname:'Linux'}});
  applyEvent(v,2,{producerId:'n',body:'workspaceRegistered',workspaceRegistered:{workspaceId:'d',nodeId:'n',projectId:'id:p',path:'/selected',gitRoot:'/selected',filesChanged:2}});
  for (const [at,id] of [[3,'a'],[5,'b']] as const) {applyEvent(v,at,{producerId:'n',body:'agentWorkspaceBound',agentWorkspaceBound:{agentId:id,workspaceId:'d'}});applyEvent(v,at+1,{producerId:'n',body:'agentStarted',agentStarted:{agentId:id,nodeId:'n',projectId:'id:p',kind:'codex'}});}
  expect(v.worktrees.size).toBe(0);expect(tiles(v).every(a=>a.shared&&a.workspace==='/selected'&&a.projectId==='id:p')).toBe(true);expect(projectChoices(v)[0]?.path).toBe('/selected');
  const restored=fromSnapshot(agora.v1.Snapshot.fromObject({pos:6,nodes:[{nodeId:'n',online:true}],workspaces:[{workspaceId:'n/d',nodeId:'n',path:'/selected',projectId:'id:p'}],agentWorkspaces:[{agentId:'n/a',workspaceId:'n/d'}],agents:[{agentId:'n/a',nodeId:'n',kind:'codex',running:true}]}));expect(tiles(restored)[0]?.workspace).toBe('/selected');
});

it('groups portable copies across machines by project without merging their sessions or folders', () => {
  const v = emptyView();
  let pos = 0;
  for (const [node, path] of [['ubuntu', '/srv/shop'], ['mac', '/Users/owner/shop']]) {
    applyEvent(v, ++pos, { producerId: node, body: 'nodeOnline', nodeOnline: { nodeId: node, hostname: node } });
    applyEvent(v, ++pos, { producerId: node, body: 'workspaceRegistered', workspaceRegistered: { workspaceId: 'folder', nodeId: node, projectId: 'id:shared', path } });
    applyEvent(v, ++pos, { producerId: node, body: 'agentWorkspaceBound', agentWorkspaceBound: { agentId: 'agent-1', workspaceId: 'folder' } });
    applyEvent(v, ++pos, { producerId: node, body: 'agentStarted', agentStarted: { agentId: 'agent-1', nodeId: node, projectId: 'id:shared', kind: 'codex' } });
  }
  const agents = tiles(v);
  expect(new Set(agents.map(a => a.projectId)).size).toBe(1);
  expect(new Set(agents.map(a => a.id)).size).toBe(2);
  expect(new Set(agents.map(a => a.workspace)).size).toBe(2);
  expect(agents.every(a => !a.shared)).toBe(true);
});

it('preserves distinct enrolled machine identities when their host names and local agent ids match', () => {
  const view = fromSnapshot(agora.v1.Snapshot.fromObject({ pos: 1,
    nodes: [{ nodeId: 'laptop', hostname: 'same-host', online: true }, { nodeId: 'desktop', hostname: 'same-host', online: true }],
    workspaces: [{ workspaceId: 'laptop/d', nodeId: 'laptop', projectId: 'id:shared', path: '/one' },
      { workspaceId: 'desktop/d', nodeId: 'desktop', projectId: 'id:shared', path: '/two' }],
    agentWorkspaces: [{ agentId: 'laptop/agent-1', workspaceId: 'laptop/d' }, { agentId: 'desktop/agent-1', workspaceId: 'desktop/d' }],
    agents: [{ agentId: 'laptop/agent-1', nodeId: 'laptop', kind: 'codex', running: true },
      { agentId: 'desktop/agent-1', nodeId: 'desktop', kind: 'codex', running: true }],
  }));
  const agents = tiles(view);
  expect(new Set(agents.map(a => a.machine)).size).toBe(1);
  expect(new Set(agents.map(machineKey)).size).toBe(2);
  expect(agents.map(a => a.id)).toEqual(['laptop/agent-1', 'desktop/agent-1']);
  expect(agents.map(a => a.workspace)).toEqual(['/one', '/two']);
});
