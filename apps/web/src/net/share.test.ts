// ADR-0122: the window builds exactly the bytes the hub and the node check (the shared vectors), and signs a word a device's key
// verifies.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { toHex } from './approval';
import { contentBytes, itemHash, parseValues, publishErrorKey, refusalWords, sharedItemsOf, sharedSourceFor, shareSignedMessage, signShareWord, type ShareItemDef } from './share';

type Vector = {
  name: string; action: 'publish' | 'remove'; item_name: string; project_id: string; kinds: string[]; issued_ms: number;
  content_hex: string; item_hash: string; hex: string;
  item: null | { name: string; kind: 'mcp_server' | 'skill'; mcp?: { command?: string; args?: string[]; env?: [string, string][]; url?: string; headers?: [string, string][] };
    skill?: { description: string; files: { path: string; content: string }[] } };
};

const doc = JSON.parse(readFileSync('../../schema/fixtures/signing/share-item-v1.json', 'utf8')) as { cases: Vector[] };

const itemOf = (v: NonNullable<Vector['item']>): ShareItemDef => (v.kind === 'mcp_server'
  ? { name: v.name, kind: 'mcp_server', mcp: {
    command: v.mcp?.command ?? '', args: v.mcp?.args ?? [], url: v.mcp?.url ?? '',
    env: (v.mcp?.env ?? []).map(([name, value]) => ({ name, value })), headers: (v.mcp?.headers ?? []).map(([name, value]) => ({ name, value })),
  } }
  : { name: v.name, kind: 'skill', skill: v.skill });

describe('shared items', () => {
  it('build the vectors byte for byte', async () => {
    expect(doc.cases.length).toBeGreaterThanOrEqual(4);
    for (const c of doc.cases) {
      if (c.item) {
        const item = itemOf(c.item);
        expect(toHex(await contentBytes(item)), c.name).toBe(c.content_hex);
        expect(await itemHash(item), c.name).toBe(c.item_hash);
      }
      expect(toHex(shareSignedMessage(c.action, c.item_name, c.item_hash, { projectId: c.project_id, kinds: c.kinds }, c.issued_ms)), c.name).toBe(c.hex);
    }
  });

  it('sign a word that the device key verifies', async () => {
    const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
    const device = { id: 'web-1', keys: pair } as never;
    const item: ShareItemDef = { name: 'db', kind: 'mcp_server', mcp: { command: 'npx', args: ['-y'], env: [], url: '', headers: [] } };
    const w = await signShareWord(device, 'db', item, { projectId: '', kinds: [] }, 7);
    expect(w.itemHash).toBe(await itemHash(item));
    const message = shareSignedMessage('publish', 'db', w.itemHash as string, { projectId: '', kinds: [] }, 7);
    expect(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, pair.publicKey, w.signature as BufferSource, message as BufferSource)).toBe(true);
    const removal = await signShareWord(device, 'db', undefined, { projectId: '', kinds: [] }, 8);
    expect(removal.itemHash).toBe('');
    expect('item' in removal).toBe(false);
  });
});

describe('the shared items source', () => {
  it('reads the list the hub sends, content included, by name', () => {
    const items = sharedItemsOf({ items: [
      { name: 'zeta', kind: 1, itemHash: 'h2', issuedMs: 5, signerDeviceId: 'web-1', scope: { projectId: 'p', kinds: ['claude-code'] },
        signed: { item: { mcp: { url: 'https://z.example', headers: [{ name: 'A', value: '1' }] } } }, refused: [{ nodeId: 'n', code: 'share.refused', reason: 'older' }] },
      { name: 'alpha', kind: 1, itemHash: 'h1', issuedMs: '7', signed: { item: { mcp: { command: 'npx', args: ['-y'] } } } },
      { name: 'odd', kind: 9 },
    ] });
    expect(items.map((i) => i.name)).toEqual(['alpha', 'zeta']);
    expect(items[0]).toMatchObject({ issuedMs: 7, scope: { projectId: '', kinds: [] }, mcp: { command: 'npx', args: ['-y'], env: [] }, refused: [] });
    expect(items[1]).toMatchObject({ scope: { projectId: 'p', kinds: ['claude-code'] }, mcp: { url: 'https://z.example' }, refused: [{ node: 'n', reason: 'older' }] });
    expect(sharedItemsOf(undefined)).toEqual([]);
  });

  it('signs a publish after the word it replaces, and a removal under the item\'s scope', async () => {
    const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
    type Sent = { shareItemPublish: { signed: Record<string, unknown> }; shareItemRemove: { signed: Record<string, unknown> } };
    const sent: Sent[] = [];
    const source = sharedSourceFor(async (body) => { sent.push(body as Sent); return {}; }, { id: 'web-1', keys: pair } as never, () => 100);
    const item: ShareItemDef = { name: 'db', kind: 'mcp_server', mcp: { command: 'npx', args: [], env: [], url: '', headers: [] } };
    expect(await source.publish(item, { projectId: '', kinds: [] }, 500)).toEqual({ ok: true });
    expect(sent[0]?.shareItemPublish.signed).toMatchObject({ name: 'db', issuedMs: 501, signerDeviceId: 'web-1' });
    await source.remove({ name: 'db', kind: 'mcp_server', hash: 'x', issuedMs: 501, signer: 'web-1', scope: { projectId: 'p', kinds: [] }, refused: [] });
    expect(sent[1]?.shareItemRemove.signed).toMatchObject({ name: 'db', issuedMs: 502, itemHash: '', scope: { projectId: 'p' } });
    expect(sent[1]?.shareItemRemove.signed.item).toBeUndefined();
  });

  it('gives back the hub\'s refusal as its code and reason', async () => {
    const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
    const source = sharedSourceFor(async () => { throw { code: 'share.refused', params: { reason: 'limit' } }; }, { id: 'w', keys: pair } as never);
    const r = await source.publish({ name: 'db', kind: 'mcp_server', mcp: { command: 'x', args: [], env: [], url: '', headers: [] } }, { projectId: '', kinds: [] });
    expect(r).toEqual({ ok: false, code: 'share.refused', reason: 'limit' });
    expect(publishErrorKey(r as { code: string; reason?: string })).toBe('shared-error-limit');
    expect(publishErrorKey({ code: 'approval.signature_invalid' })).toBe('shared-error-device');
    expect(publishErrorKey({ code: 'auth.forbidden' })).toBe('shared-error-owner');
  });

  it('names every reason a machine gives', () => {
    expect(refusalWords({ node: 'n', code: 'share.refused', reason: 'secret_needs_launcher' }).key).toBe('shared-refused-secret-needs-launcher');
    expect(refusalWords({ node: 'n', code: 'approval.signature_invalid', reason: 'signature' }).key).toBe('shared-refused-signer-untrusted');
    expect(refusalWords({ node: 'n', code: 'share.secret_missing', reason: 'DOCS' })).toEqual({ key: 'shared-refused-secret', args: { secret: 'DOCS' } });
    expect(refusalWords({ node: 'n', code: 'share.secret_missing', reason: `A\u202eB${'x'.repeat(100)}` }).args?.secret).toBe(`A\\u{202e}B${'x'.repeat(100)}`.slice(0, 64));
    expect(refusalWords({ node: 'n', code: 'share.refused', reason: 'something_new' }).key).toBe('shared-refused-other');
  });

  it('reads environment and header lines', () => {
    expect(parseValues('A=1\n\n B = two words ', '=')).toEqual([{ name: 'A', value: '1' }, { name: 'B', value: 'two words' }]);
    expect(parseValues('Authorization: Bearer x:y', ':')).toEqual([{ name: 'Authorization', value: 'Bearer x:y' }]);
    expect(parseValues('=1', '=')).toBeUndefined();
    expect(parseValues('A=1\nA=2', '=')).toBeUndefined();
  });
});
