// ADR-0088: previews of an agent's local web server, as the window sees them. A forward is opened on a machine with this device's
// signature (`preview-action-v1`, vectors in schema/fixtures/signing/preview-action-v1.json); the hub gives its opener an entry ticket
// for the forward's own origin (a port of the hub's: ADR-0088 Amendment A), and the frame trades it for a cookie there.
import { toHex, type Device } from './approval';

const enc = new TextEncoder();
const field = (x: string) => `${enc.encode(x).length}:${x}`;

/** The signed bytes of a `preview.open` sent through the hub (`agora_protocol::preview_signing`). */
export function previewOpenMessage(node: string, port: number, expiresSeconds: number, nonce: string, expiresAtMs: number): Uint8Array {
  return Uint8Array.from([...enc.encode('preview-action-v1'), 0,
    ...enc.encode(field(node) + field('open') + field(String(port)) + field(String(expiresSeconds)) + field(nonce) + field(String(expiresAtMs)))]);
}

/** The `PreviewAuthorization` an open carries, signed by this device for two minutes. */
export async function signPreviewOpen(device: Device, node: string, port: number, expiresSeconds: number, now = Date.now()) {
  const nonce = toHex(crypto.getRandomValues(new Uint8Array(16)));
  const expiresAtMs = now + 120_000;
  const message = previewOpenMessage(node, port, expiresSeconds, nonce, expiresAtMs);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, message as BufferSource));
  return { deviceId: device.id, nonce, expiresAtMs, signature };
}

/** One forward as a machine announced or listed it. */
export interface Forward {
  node: string;
  id: string;
  port: number;
  openedMs: number;
  endsMs: number;
  /** The device that opened it (`owner`: on the machine itself). */
  creator: string;
}

/** "My app is on port N", from an agent or its machine: a card with an Open button, never an open. The note is the agent's words. */
export interface PreviewSuggestion { node: string; agentId: string; port: number; note: string; atMs: number }

type ForwardWire = { forwardId?: string | null; port?: number | null; openedAtMs?: unknown; expiresAtMs?: unknown; creator?: string | null; state?: number | null };
const num = (v: unknown) => (typeof v === 'number' ? v : Number(v ?? 0) || 0);

/** A forward from its wire form; `undefined` when it is not open (a `PreviewChanged` that says it ended) or cannot be read. */
export function forwardOf(node: string, f: ForwardWire | null | undefined): Forward | undefined {
  if (!f?.forwardId || f.state !== 1) return undefined;
  const port = num(f.port);
  if (port < 1 || port > 65535) return undefined;
  return { node, id: f.forwardId, port, openedMs: num(f.openedAtMs), endsMs: num(f.expiresAtMs), creator: f.creator ?? '' };
}

/** The forwards of one machine after a `PreviewChanged`: an open one is added or replaced, an ended one taken out. */
export function applyForward(list: readonly Forward[], node: string, f: ForwardWire | null | undefined): Forward[] {
  const rest = list.filter((x) => !(x.node === node && x.id === f?.forwardId));
  const open = forwardOf(node, f);
  return open ? [...rest, open] : rest;
}

/** Where the frame goes to enter a forward: the hub's own host, the forward's port, the ticket (single use, 60 s). */
export function entryUrl(hostname: string, port: number, ticket: string): string {
  const host = hostname.includes(':') && !hostname.startsWith('[') ? `[${hostname}]` : hostname;
  return `https://${host}:${port}/.agora-preview/enter?t=${encodeURIComponent(ticket)}`;
}

/** The forward's origin, for paths typed in the address. */
export function originUrl(hostname: string, port: number, path: string): string {
  const host = hostname.includes(':') && !hostname.startsWith('[') ? `[${hostname}]` : hostname;
  const p = path.startsWith('/') ? path : `/${path}`;
  return `https://${host}:${port}${p}`;
}

/** What the hub answered to an entry: the port and the ticket, or the stable code (and reason) of a refusal. */
export type EntryResult = { ok: true; port: number; ticket: string } | { ok: false; code: string; reason: string };

/** Asks the hub for an entry ticket (`POST /api/preview/enter`), with the session's bearer token. */
export async function askEntry(token: string, node: string, forward: string, fetcher: typeof fetch = fetch): Promise<EntryResult> {
  try {
    const r = await fetcher('/api/preview/enter', {
      method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
      body: JSON.stringify({ node, forward }),
    });
    const j = (await r.json().catch(() => ({}))) as Record<string, unknown>;
    if (r.ok && typeof j.port === 'number' && typeof j.ticket === 'string') return { ok: true, port: j.port, ticket: j.ticket };
    return { ok: false, code: typeof j.code === 'string' ? j.code : r.status === 401 ? 'auth.unauthenticated' : 'unavailable', reason: typeof j.reason === 'string' ? j.reason : '' };
  } catch {
    return { ok: false, code: 'unavailable', reason: '' };
  }
}

/** The locale key of an entry's or an open's refusal. */
export function previewErrorKey(code: string, reason = ''): string {
  if (code === 'preview.unavailable' && reason === 'no_origin') return 'preview-error-no-origin';
  if (code === 'preview.port_refused') return reason === 'node_port' ? 'preview-error-node-port' : reason === 'privileged' ? 'preview-error-privileged' : reason === 'not_loopback' ? 'preview-error-not-listening' : 'preview-error-port';
  const known: Record<string, string> = {
    'preview.unknown': 'preview-error-unknown', 'preview.expired': 'preview-error-unknown', 'preview.limit': 'preview-error-limit',
    'auth.forbidden': 'preview-error-forbidden', 'approval.signature_invalid': 'preview-error-untrusted', 'node.policy_denied': 'preview-error-untrusted',
    'protocol.capability_missing': 'preview-error-old', 'node.offline': 'preview-error-offline',
  };
  return known[code] ?? 'preview-error-other';
}
