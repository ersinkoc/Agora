// ADR-0057: helpers under the agent that started them, the start request as shown, and the decision's signed bytes.
import { describe, expect, it } from 'vitest';
import { helperTree } from '../model';
import { signStart, startSignedMessage, thisDevice, toHex, type KeyStore } from './approval';
import { applyEvent, approvalList, emptyView, fromSnapshot, startRequest, tiles } from './view';

function memoryStore(): KeyStore {
  let saved: Awaited<ReturnType<KeyStore['load']>>;
  return { load: async () => saved, save: async (d) => { saved = d; } };
}

describe('agents that start agents', () => {
  it('knows each helper\'s parent, from events and from a snapshot', () => {
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'n', body: 'agentStarted', agentStarted: { agentId: 'a', nodeId: 'n', kind: 'claude' } });
    applyEvent(v, 2, { producerId: 'n', body: 'agentStarted', agentStarted: { agentId: 'b', nodeId: 'n', kind: 'codex', parentAgentId: 'a' } });
    const t = tiles(v);
    const b = t.find((x) => x.id === 'n/b');
    const a = t.find((x) => x.id === 'n/a');
    expect(b?.parentId).toBe('n/a');
    expect(b?.parentName).toBe(a?.name);
    expect(a?.parentId).toBeUndefined();
    const s = fromSnapshot({ agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'claude' }, { agentId: 'n/b', nodeId: 'n', kind: 'codex', parentAgentId: 'n/a' }] });
    expect(tiles(s).find((x) => x.id === 'n/b')?.parentId).toBe('n/a');
  });

  it('puts helpers right under their parent', () => {
    const list = [{ id: 'c', parentId: 'b' }, { id: 'x' }, { id: 'b', parentId: 'a' }, { id: 'a' }, { id: 'orphan', parentId: 'gone' }];
    expect(helperTree(list).map((e) => `${e.agent.id}:${e.depth}`)).toEqual(['x:0', 'a:0', 'b:1', 'c:2', 'orphan:0']);
  });

  it('reads a start request and leaves other approvals alone', () => {
    const detail = JSON.stringify({ kind: 'codex', folder: '/w', prompt: 'fix it', inside: true, worktree: false, depth: 1 });
    expect(startRequest('agent.start', detail)).toEqual({ kind: 'codex', folder: '/w', prompt: 'fix it', inside: true, worktree: false, depth: 1 });
    expect(startRequest('shell', detail)).toBeUndefined();
    expect(startRequest('agent.start', 'not json')).toBeUndefined();
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'n', body: 'approvalRequested', approvalRequested: { approvalId: 'ap', agentId: 'a', action: 'agent.start', detail } });
    const [a] = approvalList(v);
    expect(a?.start?.prompt).toBe('fix it');
    expect(a?.detail).toBe(detail); // kept byte for byte: the decision is signed over it
  });

  it('signs the bytes the node rebuilds, bound to the request and the grant', async () => {
    const detail = '{"prompt":"x"}';
    const m = await startSignedMessage('node-1', 'ap-1', true, 3, false, detail);
    const hash = toHex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(detail))));
    const want = ['agora-approval-start-v1', 'node-1', 'ap-1', '1', '3', '0', hash].join('\u0000');
    expect(new TextDecoder().decode(m)).toBe(want);
    const d = await thisDevice(memoryStore());
    const sig = await signStart(d, 'node-1', 'ap-1', true, 3, false, detail);
    const verify = async (msg: Uint8Array) => crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, d.keys.publicKey, sig as BufferSource, msg as BufferSource);
    expect(await verify(m)).toBe(true);
    expect(await verify(await startSignedMessage('node-1', 'ap-1', true, 0, false, detail))).toBe(false);
    expect(await verify(await startSignedMessage('node-1', 'ap-1', true, 3, false, '{"prompt":"y"}'))).toBe(false);
  });
});
