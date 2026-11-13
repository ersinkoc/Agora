// A person's devices (ADR-0061): this browser registers its key and a name with the hub when it signs in; another device of the same
// person that the machines already trust can approve it (an endorsement, signed with that device's key and verified by each machine);
// a trusted device can remove one (a revocation). The hub only carries these; its list of devices is only a list of candidates.
// The signed bytes are the node's (crates/agora-node/src/device_trust.rs); vectors: schema/fixtures/signing/device-trust-v1.json.

import type { agora } from '../gen/agora.js';
import { toHex, type Device } from './approval';

export type DeviceInfo = agora.v1.IDeviceInfo;

// An endorsement lasts at most this long (the machine renews it each time the device is used there).
export const ENDORSEMENT_MS = 90 * 24 * 60 * 60 * 1000;

const enc = new TextEncoder();

function field(out: number[], x: string) {
  const b = enc.encode(x);
  out.push(...enc.encode(`${b.length}:`), ...b);
}

export interface EndorsementFields {
  deviceId: string;
  publicKey: Uint8Array;
  name: string;
  nodes: string[];
  issuedAtMs: number;
  expiresAtMs: number;
  endorserDeviceId: string;
}

export function endorseMessage(e: EndorsementFields): Uint8Array {
  const out = [...enc.encode('agora-device-endorse-v1'), 0];
  field(out, e.deviceId);
  field(out, toHex(e.publicKey));
  field(out, e.name);
  field(out, String(e.nodes.length));
  for (const n of e.nodes) field(out, n);
  field(out, String(e.issuedAtMs));
  field(out, String(e.expiresAtMs));
  field(out, e.endorserDeviceId);
  return Uint8Array.from(out);
}

export function revokeMessage(deviceId: string, issuedAtMs: number, revokerDeviceId: string): Uint8Array {
  const out = [...enc.encode('agora-device-revoke-v1'), 0];
  field(out, deviceId);
  field(out, String(issuedAtMs));
  field(out, revokerDeviceId);
  return Uint8Array.from(out);
}

export function fromHex(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

// What people compare: the first 12 bytes (96 bits) of the SHA-256 of the 65-byte key, in groups of 4 hex digits joined by `-`.
// Always computed here from the key bytes, never taken from the hub.
export async function fingerprint(publicKey: Uint8Array, subtle: SubtleCrypto = crypto.subtle): Promise<string> {
  const d = toHex(new Uint8Array(await subtle.digest('SHA-256', publicKey as BufferSource)).slice(0, 12));
  return [0, 4, 8, 12, 16, 20].map((i) => d.slice(i, i + 4)).join('-');
}

// Proof of possession (ADR-0061): what this device signs to register itself, and to ask another device to approve it.
export function registerMessage(deviceId: string, name: string, publicKey: Uint8Array, signedAtMs: number): Uint8Array {
  const out = [...enc.encode('agora-device-register-v1'), 0];
  field(out, deviceId);
  field(out, name);
  field(out, toHex(publicKey));
  field(out, String(signedAtMs));
  return Uint8Array.from(out);
}

export function askMessage(deviceId: string, signedAtMs: number): Uint8Array {
  const out = [...enc.encode('agora-device-ask-v1'), 0];
  field(out, deviceId);
  field(out, String(signedAtMs));
  return Uint8Array.from(out);
}

const sign = async (device: Device, m: Uint8Array, subtle: SubtleCrypto) =>
  new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, m as BufferSource));

export async function signRegistration(device: Device, name: string, nowMs: number, subtle: SubtleCrypto = crypto.subtle): Promise<agora.v1.IDeviceRegister> {
  const publicKey = fromHex(device.publicKeyHex);
  return { deviceId: device.id, name, publicKey, signedAtMs: nowMs, signature: await sign(device, registerMessage(device.id, name, publicKey, nowMs), subtle) };
}

// Did the holder of the device's key register it as listed? The card for a device that asks is shown only then: a hub cannot make up
// a request for a key nobody holds.
export async function registrationVerifies(d: DeviceInfo, subtle: SubtleCrypto = crypto.subtle): Promise<boolean> {
  const key = d.publicKey, sig = d.registrationSignature;
  if (!key || key.length !== 65 || !sig || sig.length !== 64 || !d.deviceId) return false;
  try {
    const k = await subtle.importKey('raw', key as BufferSource, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
    return await subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, k, sig as BufferSource, registerMessage(d.deviceId, d.name ?? '', key, Number(d.signedAtMs ?? 0)) as BufferSource);
  } catch { return false; }
}

export async function signAsk(device: Device, nowMs: number, subtle: SubtleCrypto = crypto.subtle): Promise<agora.v1.IDeviceAskApproval> {
  return { deviceId: device.id, signedAtMs: nowMs, signature: await sign(device, askMessage(device.id, nowMs), subtle) };
}

// The name this browser registers under: the browser and the system, as the person would say it ("Firefox on Linux").
export function browserName(ua: string = typeof navigator === 'undefined' ? '' : navigator.userAgent): string {
  const browser = /Edg\//.test(ua) ? 'Edge' : /OPR\//.test(ua) ? 'Opera' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Browser';
  const system = /Android/.test(ua) ? 'Android' : /iPhone|iPad/.test(ua) ? 'iOS' : /Windows/.test(ua) ? 'Windows' : /Mac OS X|Macintosh/.test(ua) ? 'macOS' : /Linux/.test(ua) ? 'Linux' : '';
  return system ? `${browser} · ${system}` : browser;
}

// Signs, with this device's key, that `target` is the person's too, for `nodes` (empty: every machine of theirs).
export async function signEndorsement(device: Device, target: { deviceId: string; publicKey: Uint8Array; name: string }, nodes: string[], nowMs: number,
  subtle: SubtleCrypto = crypto.subtle): Promise<agora.v1.IDeviceEndorsement> {
  const fields: EndorsementFields = { ...target, nodes, issuedAtMs: nowMs, expiresAtMs: nowMs + ENDORSEMENT_MS, endorserDeviceId: device.id };
  const signature = new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, endorseMessage(fields) as BufferSource));
  return { ...fields, signature };
}

export async function signRevocation(device: Device, deviceId: string, nowMs: number, subtle: SubtleCrypto = crypto.subtle): Promise<agora.v1.IDeviceRevocation> {
  const signature = new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, revokeMessage(deviceId, nowMs, device.id) as BufferSource));
  return { deviceId, issuedAtMs: nowMs, revokerDeviceId: device.id, signature };
}

// The machines that say they trust `deviceId` (each node reports its trusted devices, ADR-0061).
export function trustedBy(trusted: ReadonlyMap<string, readonly string[]> | undefined, deviceId: string): string[] {
  if (!trusted) return [];
  return [...trusted.entries()].filter(([, ids]) => ids.includes(deviceId)).map(([node]) => node).sort();
}

// Whether another device of mine that `node` trusts could approve this one there: then the window offers to ask instead of a command.
export function canBeApprovedOn(trusted: ReadonlyMap<string, readonly string[]> | undefined, node: string, mine: readonly DeviceInfo[], me: string): boolean {
  const ids = trusted?.get(node) ?? [];
  return mine.some((d) => d.deviceId && d.deviceId !== me && !Number(d.revokedMs ?? 0) && ids.includes(d.deviceId));
}

// The person's devices that wait for this one to approve them.
export function waitingForApproval(mine: readonly DeviceInfo[], me: string): DeviceInfo[] {
  return mine.filter((d) => d.deviceId !== me && Number(d.approvalAskedMs ?? 0) > 0 && !Number(d.revokedMs ?? 0));
}
