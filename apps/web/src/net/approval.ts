// Approvals are answered with a signature from this device's own key (ADR-0020, ADR-0028): ECDSA P-256 over the exact
// message the node rebuilds (crates/agora-node/src/approvals.rs `signed_message`). The private key never leaves the
// browser (non-extractable); the node only accepts it after its owner ran `agora node trust` with the public key.

export function signedMessage(nodeId: string, approvalId: string, approved: boolean): Uint8Array {
  const enc = new TextEncoder();
  const parts = [enc.encode('agora-approval-v1'), enc.encode(nodeId), enc.encode(approvalId)];
  const out: number[] = [];
  parts.forEach((p, i) => {
    if (i > 0) out.push(0);
    out.push(...p);
  });
  out.push(0, approved ? 1 : 0);
  return Uint8Array.from(out);
}

// ADR-0057: a decision on an agent's request to start another is signed over the request exactly as it was shown (a SHA-256 of its
// detail) and the grant that comes with it, so a hub can neither swap the prompt nor add or strip a grant
// (crates/agora-node/src/approvals.rs `start_signed_message`).
export async function startSignedMessage(
  nodeId: string, approvalId: string, approved: boolean, helperGrant: number, helpersMayStart: boolean, detail: string,
  subtle: SubtleCrypto = crypto.subtle,
): Promise<Uint8Array> {
  const enc = new TextEncoder();
  const hash = toHex(new Uint8Array(await subtle.digest('SHA-256', enc.encode(detail) as BufferSource)));
  // ADR-0066: a request that lists settings is decided only over `agora-approval-start-v2`, which names them (length-prefixed) before the
  // hash; one without keeps v1 exactly. The settings are read from the same detail that is shown and hashed.
  const settings = detailSettings(detail);
  const parts = settings.length > 0
    ? ['agora-approval-start-v2', nodeId, approvalId, approved ? '1' : '0', String(helperGrant), helpersMayStart ? '1' : '0', signedPairs(settings), hash]
    : ['agora-approval-start-v1', nodeId, approvalId, approved ? '1' : '0', String(helperGrant), helpersMayStart ? '1' : '0', hash];
  const out: number[] = [];
  parts.forEach((p, i) => {
    if (i > 0) out.push(0);
    out.push(...enc.encode(p));
  });
  return Uint8Array.from(out);
}

/** ADR-0066: `[id, value]` pairs as signed: the count, then `:<id byte length>:<id>:<value byte length>:<value>` per pair in the order given (UTF-8
 * lengths), so a list can be neither added to, changed nor reordered without changing the bytes. */
export function signedPairs(items: readonly (readonly [string, string])[]): string {
  const n = (x: string) => new TextEncoder().encode(x).length;
  return String(items.length) + items.map(([id, value]) => `:${n(id)}:${id}:${n(value)}:${value}`).join('');
}

/** ADR-0066: the settings a start request's detail lists, read as the node reads them (a `settings` list of `{id, value}`). */
export function detailSettings(detail: string): [string, string][] {
  try {
    const d = JSON.parse(detail) as { settings?: unknown };
    if (!Array.isArray(d.settings)) return [];
    return d.settings.map((x) => {
      const o = (typeof x === 'object' && x !== null ? x : {}) as Record<string, unknown>;
      return [typeof o.id === 'string' ? o.id : '', typeof o.value === 'string' ? o.value : ''] as [string, string];
    });
  } catch {
    return [];
  }
}

/** Why an answer to an approval did not go through, as one of the web's own texts. `auth.forbidden` is lasting (this device or this
 * person may not answer for that project), so it never says to try again. */
export function approvalFailureKey(code: string | undefined): string {
  switch (code) {
    case 'approval.signature_invalid': return 'approval-not-trusted';
    case 'auth.forbidden': return 'approval-forbidden';
    // ADR-0090 Amendment A: the run the request named is gone (the agent ended or restarted), or the request ran out of time.
    case 'agent.not_found': return 'approval-no-longer-valid';
    case 'approval.expired': return 'approval-ran-out';
    default: return 'approval-failed';
  }
}

export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export interface Device {
  id: string;
  keys: CryptoKeyPair;
  publicKeyHex: string; // 65-byte uncompressed SEC1 point, the form `agora node trust` takes
}

// Where the key pair lives between visits. IndexedDB can hold CryptoKey objects without exposing the private bytes.
export interface KeyStore {
  load(): Promise<{ id: string; keys: CryptoKeyPair } | undefined>;
  save(d: { id: string; keys: CryptoKeyPair }): Promise<void>;
}

const ALGORITHM = { name: 'ECDSA', namedCurve: 'P-256' } as const;

export async function thisDevice(store: KeyStore, subtle: SubtleCrypto = crypto.subtle): Promise<Device> {
  let saved = await store.load();
  if (!saved) {
    const keys = await subtle.generateKey(ALGORITHM, false, ['sign', 'verify']);
    // 64 random bits (ids made before ADR-0061's review have 24 and stay valid: they are read back as stored).
    const suffix = toHex(crypto.getRandomValues(new Uint8Array(8)));
    saved = { id: `web-${suffix}`, keys };
    await store.save(saved);
  }
  const raw = new Uint8Array(await subtle.exportKey('raw', saved.keys.publicKey));
  return { id: saved.id, keys: saved.keys, publicKeyHex: toHex(raw) };
}

// WebCrypto returns the 64-byte r||s form the node verifies.
export async function signApproval(
  device: Device,
  nodeId: string,
  approvalId: string,
  approved: boolean,
  subtle: SubtleCrypto = crypto.subtle,
): Promise<Uint8Array> {
  const sig = await subtle.sign(
    { name: 'ECDSA', hash: 'SHA-256' },
    device.keys.privateKey,
    signedMessage(nodeId, approvalId, approved) as BufferSource,
  );
  return new Uint8Array(sig);
}

// The signature of a decision on a start request (ADR-0057).
export async function signStart(
  device: Device, nodeId: string, approvalId: string, approved: boolean, helperGrant: number, helpersMayStart: boolean, detail: string,
  subtle: SubtleCrypto = crypto.subtle,
): Promise<Uint8Array> {
  const message = await startSignedMessage(nodeId, approvalId, approved, helperGrant, helpersMayStart, detail, subtle);
  return new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, message as BufferSource));
}

export function indexedDbStore(): KeyStore | undefined {
  if (typeof indexedDB === 'undefined') return undefined;
  const open = () =>
    new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open('agora-device', 1);
      r.onupgradeneeded = () => r.result.createObjectStore('keys');
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
  const run = <T,>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>) =>
    open().then(
      (db) =>
        new Promise<T>((resolve, reject) => {
          const req = f(db.transaction('keys', mode).objectStore('keys'));
          req.onsuccess = () => resolve(req.result);
          req.onerror = () => reject(req.error);
        }),
    );
  return {
    load: () => run('readonly', (s) => s.get('device')) as Promise<{ id: string; keys: CryptoKeyPair } | undefined>,
    save: async (d) => { await run('readwrite', (s) => s.put(d, 'device')); },
  };
}
