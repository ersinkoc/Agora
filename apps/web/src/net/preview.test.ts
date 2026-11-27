// ADR-0088: the window opens a forward with exactly the bytes the node checks (the shared vectors), reads what machines announce, and
// enters a forward through the hub's ticket.
import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { toHex } from './approval';
import { applyForward, askEntry, entryUrl, forwardOf, originUrl, previewErrorKey, previewOpenMessage, signPreviewOpen } from './preview';

type Vector = { name: string; node: string; port: number; expires_seconds: number; nonce: string; expires: number; hex: string };
const doc = JSON.parse(readFileSync('../../schema/fixtures/signing/preview-action-v1.json', 'utf8')) as { cases: Vector[] };

describe('previews', () => {
  it('sign the bytes of the shared vectors', () => {
    expect(doc.cases.length).toBeGreaterThanOrEqual(3);
    for (const c of doc.cases) expect(toHex(previewOpenMessage(c.node, c.port, c.expires_seconds, c.nonce, c.expires)), c.name).toBe(c.hex);
  });

  it('sign an open the device key verifies, for two minutes', async () => {
    const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
    const a = await signPreviewOpen({ id: 'web-1', keys: pair } as never, 'mac', 5173, 3600, 1_000);
    expect(a.deviceId).toBe('web-1');
    expect(a.expiresAtMs).toBe(121_000);
    const ok = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, pair.publicKey, a.signature as BufferSource,
      previewOpenMessage('mac', 5173, 3600, a.nonce, a.expiresAtMs) as BufferSource);
    expect(ok).toBe(true);
  });

  it('keep the open forwards a machine announces, and drop one that ended', () => {
    const open = { forwardId: 'fwd-1', port: 5173, openedAtMs: 1, expiresAtMs: '9', creator: 'web-1', state: 1 };
    let list = applyForward([], 'mac', open);
    expect(list).toEqual([{ node: 'mac', id: 'fwd-1', port: 5173, openedMs: 1, endsMs: 9, creator: 'web-1' }]);
    list = applyForward(list, 'pc', { ...open, port: 3000 });
    expect(list).toHaveLength(2);
    list = applyForward(list, 'mac', { ...open, state: 2 });
    expect(list.map((f) => f.node)).toEqual(['pc']);
    expect(forwardOf('mac', { ...open, port: 0 })).toBeUndefined();
  });

  it("enter through the hub's own host and the forward's port", async () => {
    expect(entryUrl('hub.example', 7451, 'ab')).toBe('https://hub.example:7451/.agora-preview/enter?t=ab');
    expect(entryUrl('::1', 7451, 'ab')).toBe('https://[::1]:7451/.agora-preview/enter?t=ab');
    expect(originUrl('hub.example', 7451, 'app?x=1')).toBe('https://hub.example:7451/app?x=1');
    const fetcher = vi.fn(async () => new Response(JSON.stringify({ port: 7451, ticket: 't' }), { status: 200 }));
    expect(await askEntry('tok', 'mac', 'fwd-1', fetcher as unknown as typeof fetch)).toEqual({ ok: true, port: 7451, ticket: 't' });
    const [url, init] = fetcher.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('/api/preview/enter');
    expect((init.headers as Record<string, string>).authorization).toBe('Bearer tok');
    expect(JSON.parse(init.body as string)).toEqual({ node: 'mac', forward: 'fwd-1' });
    const refused = vi.fn(async () => new Response(JSON.stringify({ code: 'preview.unavailable', reason: 'no_origin' }), { status: 409 }));
    const r = await askEntry('tok', 'mac', 'fwd-1', refused as unknown as typeof fetch);
    expect(r).toEqual({ ok: false, code: 'preview.unavailable', reason: 'no_origin' });
    expect(previewErrorKey('preview.unavailable', 'no_origin')).toBe('preview-error-no-origin');
    expect(previewErrorKey('preview.port_refused', 'not_loopback')).toBe('preview-error-not-listening');
    expect(previewErrorKey('something.new')).toBe('preview-error-other');
  });
});
