// Approvals are answered with a signature from this device's own key (ADR-0020, ADR-0028): ECDSA P-256 over the exact
// message the node rebuilds (crates/agora-node/src/approvals.rs `signed_message`). The private key never leaves the
// browser (non-extractable); the node only accepts it after its owner ran `agora node trust` with the public key.
import { ROLE_REFUSAL } from './startRefusal';

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
  // ADR-0059 decision 5: one that lists launch options (a helper that acts without asking) is decided only over
  // `agora-approval-start-v3`, which names the settings and then the option ids (length-prefixed) before the hash.
  const settings = detailSettings(detail);
  const options = detailLaunchOptions(detail).map((o) => o.id);
  const head = [nodeId, approvalId, approved ? '1' : '0', String(helperGrant), helpersMayStart ? '1' : '0'];
  const parts = options.length > 0
    ? ['agora-approval-start-v3', ...head, signedPairs(settings), signedList(options), hash]
    : settings.length > 0
      ? ['agora-approval-start-v2', ...head, signedPairs(settings), hash]
      : ['agora-approval-start-v1', ...head, hash];
  const out: number[] = [];
  parts.forEach((p, i) => {
    if (i > 0) out.push(0);
    out.push(...enc.encode(p));
  });
  return Uint8Array.from(out);
}

/** ADR-0101: the bytes of an "always" answer (`ApprovalResolve.scope` = ALWAYS, approved): `agora-approval-always-v1`, one NUL, then
 * F(node_id) F(approval_id) F(verdict) F(agent_id) F(sha256(action)) F(sha256(detail)) F(always_label) F(always_option_id), where F(x) is
 * the UTF-8 byte length of x, `:`, then x (crates/agora-node/src/approvals.rs, vectors in schema/fixtures/signing/approval-always-v1.json).
 * `agentId` is the node's own id of the agent (no `<node>/` prefix); every text is used exactly as the machine announced it. */
export async function alwaysSignedMessage(
  nodeId: string, approvalId: string, agentId: string, action: string, detail: string, alwaysLabel: string, alwaysOptionId: string,
  subtle: SubtleCrypto = crypto.subtle,
): Promise<Uint8Array> {
  const enc = new TextEncoder();
  const sha = async (x: string) => toHex(new Uint8Array(await subtle.digest('SHA-256', enc.encode(x) as BufferSource)));
  const fields = [nodeId, approvalId, '1', agentId, await sha(action), await sha(detail), alwaysLabel, alwaysOptionId];
  const body = fields.map((f) => `${enc.encode(f).length}:${f}`).join('');
  return Uint8Array.from([...enc.encode('agora-approval-always-v1'), 0, ...enc.encode(body)]);
}

/** ADR-0066: `[id, value]` pairs as signed: the count, then `:<id byte length>:<id>:<value byte length>:<value>` per pair in the order given (UTF-8
 * lengths), so a list can be neither added to, changed nor reordered without changing the bytes. */
export function signedPairs(items: readonly (readonly [string, string])[]): string {
  const n = (x: string) => new TextEncoder().encode(x).length;
  return String(items.length) + items.map(([id, value]) => `:${n(id)}:${id}:${n(value)}:${value}`).join('');
}

/** ADR-0059: ids as signed: the count, then `:<byte length>:<id>` per id in the order given (UTF-8 lengths). */
export function signedList(items: readonly string[]): string {
  const n = (x: string) => new TextEncoder().encode(x).length;
  return String(items.length) + items.map((id) => `:${n(id)}:${id}`).join('');
}

export interface StartOption { id: string; label: string; skipsPermissions: boolean }

/** ADR-0059 decision 5: the launch options a start request's detail lists, read as the node reads them (a `launch_options` list of
 * `{id, label, skips_permissions}`). Whether one skips permissions is the node's word, bound by the signed hash of the detail. */
export function detailLaunchOptions(detail: string): StartOption[] {
  try {
    const d = JSON.parse(detail) as { launch_options?: unknown };
    if (!Array.isArray(d.launch_options)) return [];
    return d.launch_options.map((x) => {
      const o = (typeof x === 'object' && x !== null ? x : {}) as Record<string, unknown>;
      return {
        id: typeof o.id === 'string' ? o.id : '',
        label: typeof o.label === 'string' ? o.label : '',
        skipsPermissions: o.skips_permissions === true,
      };
    });
  } catch {
    return [];
  }
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

/** Why an answer to an approval did not go through, as one of the web's own texts. `auth.forbidden` (this device may not answer for
 * that project) and `ROLE_REFUSAL` (this person's role may not, #1044) are lasting, so they never say to try again. */
export function approvalFailureKey(code: string | undefined): string {
  switch (code) {
    case 'approval.signature_invalid': return 'approval-not-trusted';
    case 'auth.forbidden': return 'approval-forbidden';
    case ROLE_REFUSAL: return 'approval-role';
    // ADR-0090 Amendment A: the run the request named is gone (the agent ended or restarted), or the request ran out of time.
    case 'agent.not_found': return 'approval-no-longer-valid';
    case 'approval.expired': return 'approval-ran-out';
    // ADR-0101: the machine's policy allows single answers only, the agent does not offer that answer, or its bridge is too old.
    case 'approval.scope_forbidden': return 'approval-scope-forbidden';
    case 'approval.scope_unavailable': return 'approval-scope-unavailable';
    case 'approval.adapter_outdated': return 'approval-adapter-outdated';
    // ADR-0110: the agent did not take the answer in time (it may still be working), or it stopped.
    case 'adapter.no_answer': return 'approval-no-answer';
    case 'adapter.agent_exited': return 'approval-agent-exited';
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

/** ADR-0101: the signature of an "always" answer (only ever an Allow). */
export async function signAlways(
  device: Device, nodeId: string, approvalId: string, agentId: string, action: string, detail: string, alwaysLabel: string, alwaysOptionId: string,
  subtle: SubtleCrypto = crypto.subtle,
): Promise<Uint8Array> {
  const message = await alwaysSignedMessage(nodeId, approvalId, agentId, action, detail, alwaysLabel, alwaysOptionId, subtle);
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
