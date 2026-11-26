// ADR-0057: helpers under the agent that started them, the start request as shown, and the decision's signed bytes.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { helperTree } from '../model';
import { detailLaunchOptions, signStart, startSignedMessage, thisDevice, toHex, type KeyStore } from './approval';
import { HIDDEN_RANGES, applyEvent, approvalList, emptyView, fromSnapshot, startRequest, tiles, visible } from './view';

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

  it('shows hidden and control characters as visible escapes, the same set the node refuses', () => {
    expect(visible('a\u200bb\u202ec\u{e0041}d\ufe0fe\u3164f\u2800g\u00adh\u001bi\nj\tk')).toBe('a\\u{200b}b\\u{202e}c\\u{e0041}d\\u{fe0f}e\\u{3164}f\\u{2800}g\\u{ad}h\\u{1b}i\nj\tk');
    expect(visible('Grüße 🚀 çalış')).toBe('Grüße 🚀 çalış');
  });

  it('hides exactly the shared list the node and the TUI use', () => {
    const file = readFileSync(new URL('../../../../crates/agora-core/src/hidden-characters.txt', import.meta.url), 'utf8')
      .split('\n').map((l) => l.trim()).filter((l) => l && !l.startsWith('#'))
      .map((l) => { const [lo, hi] = l.split('-'); return [parseInt(lo as string, 16), parseInt((hi ?? lo) as string, 16)]; });
    expect(HIDDEN_RANGES.map((r) => [...r])).toEqual(file);
    for (const [lo, hi] of HIDDEN_RANGES) {
      for (const c of [lo, hi]) expect(visible(String.fromCodePoint(c))).toBe(`\\u{${c.toString(16)}}`);
    }
    expect(visible('\u2028\u2029\ue000')).toBe('\\u{2028}\\u{2029}\\u{e000}');
  });

  it('puts helpers right under their parent', () => {
    const list = [{ id: 'c', parentId: 'b' }, { id: 'x' }, { id: 'b', parentId: 'a' }, { id: 'a' }, { id: 'orphan', parentId: 'gone' }];
    expect(helperTree(list).map((e) => `${e.agent.id}:${e.depth}`)).toEqual(['x:0', 'a:0', 'b:1', 'c:2', 'orphan:0']);
  });

  it('reads a start request and leaves other approvals alone', () => {
    const detail = JSON.stringify({ kind: 'codex', folder: '/w', prompt: 'fix it', inside: true, worktree: false, depth: 1, grant_allowed: true });
    expect(startRequest('agent.start', detail)).toEqual({ agent: '', agentKind: '', node: '', kind: 'codex', folder: '/w', prompt: 'fix it', inside: true, worktree: false, depth: 1, grantAllowed: true, settings: [], options: [] });
    // A shell (or an older node) offers no grant.
    expect(startRequest('agent.start', JSON.stringify({ kind: 'shell', inside: true }))?.grantAllowed).toBe(false);
    expect(startRequest('shell', detail)).toBeUndefined();
    expect(startRequest('agent.start', 'not json')).toBeUndefined();
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'n', body: 'approvalRequested', approvalRequested: { approvalId: 'ap', agentId: 'a', action: 'agent.start', detail } });
    const [a] = approvalList(v);
    expect(a?.start?.prompt).toBe('fix it');
    expect(a?.detail).toBe(detail); // kept byte for byte: the decision is signed over it
    // No name is known for the agent or the machine: empty, never the id (the window says it in words).
    expect(a?.agentName).toBe('');
    expect(a?.machineName).toBe('');
    // With the machine announced, its host name is the machine's name.
    const w = emptyView();
    applyEvent(w, 1, { producerId: 'n', body: 'nodeOnline', nodeOnline: { nodeId: 'n', hostname: 'Laptop', os: 'linux', build: '', binary: '' } } as never);
    applyEvent(w, 2, { producerId: 'n', body: 'approvalRequested', approvalRequested: { approvalId: 'ap', agentId: 'a', action: 'agent.start', detail } });
    expect(approvalList(w)[0]?.machineName).toBe('Laptop');
  });

  it('hashes the announced detail byte for byte (the vector the node and the TUI test too)', async () => {
    // crates/agora-node/src/approvals.rs START_VECTOR_DETAIL / START_VECTOR_HASH: non-ASCII, a raw line break and an emoji.
    const detail = '{"prompt":"Grüße\nçalış 🚀"}';
    const m = await startSignedMessage('node-1', 'ap-1', true, 3, false, detail);
    expect(new TextDecoder().decode(m)).toBe(['agora-approval-start-v1', 'node-1', 'ap-1', '1', '3', '0', '2705e6e894314e40e3938d6e7780386d393351c5d33a5826ed13948b56d8c076'].join('\u0000'));
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

// ADR-0066: a start request that lists settings (model, effort) is decided over `agora-approval-start-v2`.
describe('settings on a start request', () => {
  const vectors = JSON.parse(readFileSync('../../schema/fixtures/signing/approval-start-v2.json', 'utf8')) as { cases: { name: string; node: string; approval_id: string; approved: boolean; helper_grant: number; helpers_may_start: boolean; settings: [string, string][]; detail: string; hex: string }[] };

  it('signs the vectors the node checks too', async () => {
    for (const c of vectors.cases) {
      const m = await startSignedMessage(c.node, c.approval_id, c.approved, c.helper_grant, c.helpers_may_start, c.detail);
      expect(toHex(m), c.name).toBe(c.hex);
    }
  });

  it('reads them from the detail for the card, in the order the node listed them', () => {
    const detail = vectors.cases[0]!.detail;
    const s = startRequest('agent.start', detail)?.settings;
    expect(s).toEqual([{ id: 'model', label: 'setting-model', value: 'gpt-6.1-sol' }, { id: 'effort', label: 'setting-effort', value: 'high' }]);
    expect(startRequest('agent.start', '{"kind":"codex"}')?.settings).toEqual([]);
    // Not a list, or not strings: nothing to show and nothing to sign (the node reads them the same way).
    expect(startRequest('agent.start', '{"settings":"x"}')?.settings).toEqual([]);
    expect(startRequest('agent.start', '{"settings":[{"id":1,"value":{}}]}')?.settings).toEqual([{ id: '', label: '', value: '' }]);
  });

  it('is not the message of a request without settings, and changes with every setting, its order and its value', async () => {
    const base = vectors.cases[0]!;
    const sign = (detail: string) => startSignedMessage('n', 'ap', true, 0, false, detail).then((m) => new TextDecoder().decode(m));
    const d = (settings: { id: string; value: string }[]) => JSON.stringify({ kind: 'codex', settings });
    expect((await sign(d([]))).startsWith('agora-approval-start-v1')).toBe(true);
    const one = await sign(d([{ id: 'model', value: 'a' }]));
    expect(one.startsWith('agora-approval-start-v2')).toBe(true);
    const all = new Set([
      one,
      await sign(d([{ id: 'model', value: 'b' }])),
      await sign(d([{ id: 'model', value: 'a' }, { id: 'effort', value: 'low' }])),
      await sign(d([{ id: 'effort', value: 'low' }, { id: 'model', value: 'a' }])),
      await sign(d([{ id: 'modela', value: '' }])),
    ]);
    expect(all.size).toBe(5);
    // Multi-byte text is counted in bytes, as the node does.
    expect(await sign(d([{ id: 'model', value: 'é' }]))).toContain('5:model:2:é');
    expect(base.settings.length).toBe(2);
  });
});

// ADR-0059 decision 5: a start request that lists launch options (a helper that acts without asking) is decided over
// `agora-approval-start-v3`.
describe('launch options on a start request', () => {
  const vectors = JSON.parse(readFileSync('../../schema/fixtures/signing/approval-start-v3.json', 'utf8')) as { cases: { name: string; node: string; approval_id: string; approved: boolean; helper_grant: number; helpers_may_start: boolean; settings: [string, string][]; options: string[]; detail: string; hex: string }[] };

  it('signs the vectors the node checks too', async () => {
    for (const c of vectors.cases) {
      const m = await startSignedMessage(c.node, c.approval_id, c.approved, c.helper_grant, c.helpers_may_start, c.detail);
      expect(toHex(m), c.name).toBe(c.hex);
      expect(detailLaunchOptions(c.detail).map((o) => o.id), c.name).toEqual(c.options);
    }
  });

  it('reads them for the card with whether each skips permissions, the node\'s word', () => {
    const detail = JSON.stringify({ kind: 'codex', launch_options: [{ id: 'skip-permissions', label: 'option-skip-permissions', skips_permissions: true }, { id: 'x', label: 1 }] });
    expect(startRequest('agent.start', detail)?.options).toEqual([
      { id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true },
      { id: 'x', label: '', skipsPermissions: false },
    ]);
    expect(startRequest('agent.start', '{"kind":"codex"}')?.options).toEqual([]);
    expect(startRequest('agent.start', '{"launch_options":"x"}')?.options).toEqual([]);
  });

  it('is never the v1 or v2 message', async () => {
    const enc = (m: Uint8Array) => new TextDecoder().decode(m);
    const d = JSON.stringify({ kind: 'codex', settings: [{ id: 'effort', value: 'high' }], launch_options: [{ id: 'skip-permissions', skips_permissions: true }] });
    const m = enc(await startSignedMessage('n', 'ap', true, 0, false, d));
    expect(m.startsWith('agora-approval-start-v3\0')).toBe(true);
    expect(m).toContain('\x001:6:effort:4:high\x001:16:skip-permissions\x00');
  });
});
