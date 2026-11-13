import { describe, expect, it } from 'vitest';
import vectors from '../../../../schema/fixtures/signing/device-trust-v1.json';
import { thisDevice, toHex, type KeyStore } from './approval';
import { applyEvent, fromSnapshot } from './view';
import { browserName, canBeApprovedOn, endorseMessage, fingerprint, fromHex, revokeMessage, signEndorsement, trustedBy, waitingForApproval } from './devices';

function memoryStore(): KeyStore {
  let saved: Awaited<ReturnType<KeyStore['load']>>;
  return { load: async () => saved, save: async (d) => { saved = d; } };
}

const ES256 = { name: 'ECDSA', hash: 'SHA-256' } as const;

describe('device endorsements (ADR-0061)', () => {
  it('signs the bytes the node and the terminal client build, and the vectors\' signatures verify', async () => {
    const endorser = await crypto.subtle.importKey('raw', fromHex(vectors.endorser_public_key) as BufferSource, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
    for (const c of vectors.endorsements) {
      const m = endorseMessage({ deviceId: c.device_id, publicKey: fromHex(c.public_key), name: c.name, nodes: c.nodes, issuedAtMs: c.issued_at_ms, expiresAtMs: c.expires_at_ms, endorserDeviceId: c.endorser_device_id });
      expect(toHex(m), c.case).toBe(c.hex);
      expect(await crypto.subtle.verify(ES256, endorser, fromHex(c.signature) as BufferSource, m as BufferSource), c.case).toBe(true);
    }
    for (const c of vectors.revocations) {
      const m = revokeMessage(c.device_id, c.issued_at_ms, c.revoker_device_id);
      expect(toHex(m), c.case).toBe(c.hex);
      expect(await crypto.subtle.verify(ES256, endorser, fromHex(c.signature) as BufferSource, m as BufferSource)).toBe(true);
    }
  });

  it('shows the same fingerprint as every other client', async () => {
    for (const f of vectors.fingerprints) expect(await fingerprint(fromHex(f.public_key))).toBe(f.fingerprint);
  });

  it('signs an endorsement for 90 days with this browser\'s key', async () => {
    const me = await thisDevice(memoryStore());
    const other = await thisDevice(memoryStore());
    const e = await signEndorsement(me, { deviceId: other.id, publicKey: fromHex(other.publicKeyHex), name: 'Phone' }, ['laptop'], 1_700_000_000_000);
    expect(e.expiresAtMs).toBe(1_700_000_000_000 + 90 * 86_400_000);
    expect(e.endorserDeviceId).toBe(me.id);
    const m = endorseMessage({ deviceId: other.id, publicKey: fromHex(other.publicKeyHex), name: 'Phone', nodes: ['laptop'], issuedAtMs: 1_700_000_000_000, expiresAtMs: e.expiresAtMs as number, endorserDeviceId: me.id });
    expect(await crypto.subtle.verify(ES256, me.keys.publicKey, e.signature as BufferSource, m as BufferSource)).toBe(true);
  });

  it('knows which machines trust a device, when another device could approve it, and who waits', () => {
    const trusted = new Map([['laptop', ['web-old']], ['vps', ['web-old', 'web-new']], ['pi', []]]);
    expect(trustedBy(trusted, 'web-new')).toEqual(['vps']);
    expect(trustedBy(trusted, 'web-old')).toEqual(['laptop', 'vps']);
    const mine = [{ deviceId: 'web-old' }, { deviceId: 'web-new', approvalAskedMs: 5 }, { deviceId: 'web-gone', approvalAskedMs: 5, revokedMs: 9 }];
    expect(canBeApprovedOn(trusted, 'laptop', mine, 'web-new')).toBe(true);
    expect(canBeApprovedOn(trusted, 'pi', mine, 'web-new')).toBe(false);
    expect(canBeApprovedOn(trusted, 'laptop', mine, 'web-old')).toBe(false); // only itself there
    expect(waitingForApproval(mine, 'web-old').map((d) => d.deviceId)).toEqual(['web-new']);
    expect(waitingForApproval(mine, 'web-new')).toEqual([]);
  });

  it('names the browser the way a person would', () => {
    expect(browserName('Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0')).toBe('Firefox · Linux');
    expect(browserName('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15')).toBe('Safari · macOS');
    expect(browserName('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36 Edg/126.0')).toBe('Edge · Windows');
    expect(browserName('')).toBe('Browser');
  });

  it('follows which devices each machine trusts, from the snapshot and then live', () => {
    const v = fromSnapshot({ pos: 1, nodes: [{ nodeId: 'laptop', online: true, trustedDeviceIds: ['web-old'] }, { nodeId: 'vps', online: true }] });
    expect(trustedBy(v.trusted, 'web-old')).toEqual(['laptop']);
    applyEvent(v, 2, { producerId: 'vps', seq: 1, body: 'nodeTrustedDevices', nodeTrustedDevices: { deviceIds: ['web-old', 'web-new'] } } as never);
    expect(trustedBy(v.trusted, 'web-old')).toEqual(['laptop', 'vps']);
    applyEvent(v, 3, { producerId: 'laptop', seq: 1, body: 'nodeTrustedDevices', nodeTrustedDevices: { deviceIds: [] } } as never);
    expect(trustedBy(v.trusted, 'web-old')).toEqual(['vps']);
  });
});
