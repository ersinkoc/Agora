import { describe, expect, it } from 'vitest';
import { signApproval, signedMessage, thisDevice, toHex, type KeyStore } from './approval';

function memoryStore(): KeyStore {
  let saved: Awaited<ReturnType<KeyStore['load']>>;
  return { load: async () => saved, save: async (d) => { saved = d; } };
}

describe('approval signatures', () => {
  it('signs exactly the bytes the node rebuilds', () => {
    // b"agora-approval-v1" NUL "node-1" NUL "ap-1" NUL 1  (crates/agora-node/src/approvals.rs)
    const want = [...new TextEncoder().encode('agora-approval-v1'), 0, ...new TextEncoder().encode('node-1'), 0, ...new TextEncoder().encode('ap-1'), 0, 1];
    expect(Array.from(signedMessage('node-1', 'ap-1', true))).toEqual(want);
    expect(signedMessage('node-1', 'ap-1', false).at(-1)).toBe(0);
  });

  it('keeps one device key, whose raw signature verifies against its public key', async () => {
    const store = memoryStore();
    const d = await thisDevice(store);
    expect(d.id).toMatch(/^web-[0-9a-f]{16}$/); // 64 random bits
    expect(d.publicKeyHex).toHaveLength(130); // 65 bytes
    expect(d.publicKeyHex.startsWith('04')).toBe(true);
    expect((await thisDevice(store)).publicKeyHex).toBe(d.publicKeyHex); // second visit: same key

    const sig = await signApproval(d, 'node-1', 'ap-1', true);
    expect(sig).toHaveLength(64); // r || s, what the node verifies
    const ok = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, d.keys.publicKey, sig as BufferSource, signedMessage('node-1', 'ap-1', true) as BufferSource);
    expect(ok).toBe(true);
    const flipped = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, d.keys.publicKey, sig as BufferSource, signedMessage('node-1', 'ap-1', false) as BufferSource);
    expect(flipped).toBe(false);
    expect(toHex(new Uint8Array([1, 255]))).toBe('01ff');
  });

  it('never exposes the private key bytes', async () => {
    const d = await thisDevice(memoryStore());
    await expect(crypto.subtle.exportKey('pkcs8', d.keys.privateKey)).rejects.toBeTruthy();
  });
});
