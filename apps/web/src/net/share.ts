// ADR-0122: the owner's shared skills and MCP servers, as the window builds and signs them. The bytes are the hub's and the node's own
// (`agora_protocol::share`, vectors in schema/fixtures/signing/share-item-v1.json): an item's content hash, and the signed word
// `agora-share-item-v1` on a name (a publish, or a removal with an empty hash). The node verifies every word with its own trust.
import { signedList, signedPairs, toHex, type Device } from './approval';
import { visible } from './hidden';

export type ShareKind = 'mcp_server' | 'skill';
export interface ShareValue { name: string; value: string }
export interface McpServer { command: string; args: string[]; env: ShareValue[]; url: string; headers: ShareValue[] }
export interface SkillFileDef { path: string; content: string }
export interface ShareItemDef { name: string; kind: ShareKind; mcp?: McpServer; skill?: { description: string; files: SkillFileDef[] } }
export interface ShareScopeDef { projectId: string; kinds: string[] }

const enc = new TextEncoder();
const field = (x: string | Uint8Array) => {
  const b = typeof x === 'string' ? enc.encode(x) : x;
  return [...enc.encode(`${b.length}:`), ...b];
};
const byName = (a: ShareValue, b: ShareValue) => (a.name < b.name ? -1 : a.name > b.name ? 1 : a.value < b.value ? -1 : a.value > b.value ? 1 : 0);

async function sha256Hex(bytes: Uint8Array, subtle: SubtleCrypto): Promise<string> {
  return toHex(new Uint8Array(await subtle.digest('SHA-256', bytes as BufferSource)));
}

/** The bytes an item's content hash is taken over (`agora_protocol::share::content_bytes`). Sorting compares UTF-8 code units as
 *  Rust's string order does: for names of ASCII letters, digits, `_` and `-` (the only ones the rules allow) it is the same order. */
export async function contentBytes(item: ShareItemDef, subtle: SubtleCrypto = crypto.subtle): Promise<Uint8Array> {
  const out: number[] = [...enc.encode('agora-share-item-content-v1'), 0, ...field(item.kind), ...field(item.name)];
  if (item.kind === 'mcp_server') {
    const m = item.mcp ?? { command: '', args: [], env: [], url: '', headers: [] };
    const pairs = (v: ShareValue[]) => signedPairs([...v].sort(byName).map((x) => [x.name, x.value] as const));
    out.push(...field(m.command), ...field(signedList(m.args)), ...field(pairs(m.env)), ...field(m.url), ...field(pairs(m.headers)));
  } else {
    const s = item.skill ?? { description: '', files: [] };
    const files = await Promise.all(s.files.map(async (f) => [f.path, await sha256Hex(enc.encode(f.content), subtle)] as const));
    files.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : a[1] < b[1] ? -1 : 1));
    out.push(...field(s.description), ...field(signedPairs(files)));
  }
  return Uint8Array.from(out);
}

/** An item's content hash, as the signature names it (lowercase hex). */
export async function itemHash(item: ShareItemDef, subtle: SubtleCrypto = crypto.subtle): Promise<string> {
  return sha256Hex(await contentBytes(item, subtle), subtle);
}

/** The signed bytes of the owner's word on a name (`agora_protocol::share::signed_message`). */
export function shareSignedMessage(action: 'publish' | 'remove', name: string, hash: string, scope: ShareScopeDef, issuedMs: number): Uint8Array {
  return Uint8Array.from([
    ...enc.encode('agora-share-item-v1'), 0,
    ...field(action), ...field(name), ...field(hash), ...field(scope.projectId), ...field(signedList(scope.kinds)), ...field(String(issuedMs)),
  ]);
}

/** The `ShareItemSigned` a publish or a removal sends, signed by this device. `item` absent: a removal of `name`. */
export async function signShareWord(device: Device, name: string, item: ShareItemDef | undefined, scope: ShareScopeDef, issuedMs: number,
  subtle: SubtleCrypto = crypto.subtle): Promise<Record<string, unknown>> {
  const hash = item ? await itemHash(item, subtle) : '';
  const message = shareSignedMessage(item ? 'publish' : 'remove', name, hash, scope, issuedMs);
  const signature = new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.privateKey, message as BufferSource));
  return {
    ...(item ? { item: wireItem(item) } : {}),
    scope: { projectId: scope.projectId, kinds: scope.kinds },
    issuedMs, itemHash: hash, signerDeviceId: device.id, signature, name,
  };
}

/** The item as the protocol carries it (`ShareItem`; kind 1 an MCP server, 2 a skill). */
export function wireItem(item: ShareItemDef): Record<string, unknown> {
  return item.kind === 'mcp_server'
    ? { name: item.name, kind: 1, mcp: item.mcp }
    : { name: item.name, kind: 2, skill: { description: item.skill?.description ?? '', files: (item.skill?.files ?? []).map((f) => ({ path: f.path, content: enc.encode(f.content) })) } };
}

/** Whether a value names a secret (`{secret:NAME}`): such a server is not given to agents until the node's launcher comes. */
export const namesSecret = (v: string) => v.includes('{secret:');

/** What one machine said of an item it does not give to its agents (`ShareItemRefused`). */
export interface SharedRefusal { node: string; code: string; reason: string }
/** One item as the hub lists it for the owner, with its content (an MCP server's program or address). */
export interface SharedItem {
  name: string; kind: ShareKind; hash: string; scope: ShareScopeDef; issuedMs: number; signer: string; mcp?: McpServer; refused: SharedRefusal[];
  skill?: { description: string; files: SkillFileDef[] };
}
/** Where Settings → Shared reads and writes: the hub, through this window's connection, signing with this device. A failure is the
 *  stable code the hub answered (`share.refused`, `approval.signature_invalid`, ...), with its `reason` when it has one. */
export interface SharedSource {
  list(): Promise<SharedItem[]>;
  publish(item: ShareItemDef, scope: ShareScopeDef, after?: number): Promise<SharedResult>;
  remove(item: SharedItem): Promise<SharedResult>;
}
export type SharedResult = { ok: true } | { ok: false; code: string; reason?: string };

type Ask = (body: Record<string, unknown>) => Promise<Record<string, unknown>>;
const num = (v: unknown) => (typeof v === 'number' ? v : Number(v ?? 0) || 0);
const str = (v: unknown) => (typeof v === 'string' ? v : '');
const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});
const values = (v: unknown): ShareValue[] => (Array.isArray(v) ? v.map((x) => ({ name: str(obj(x).name), value: str(obj(x).value) })) : []);
// A skill file's bytes as the connection decoded them (bytes, or base64 in JSON), as text; bytes that are not UTF-8 read as nothing.
const textOf = (v: unknown): string => {
  try {
    const bytes = v instanceof Uint8Array ? v : typeof v === 'string' ? Uint8Array.from(atob(v), (c) => c.charCodeAt(0)) : Array.isArray(v) ? Uint8Array.from(v as number[]) : new Uint8Array();
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch { return ''; }
};
const kindOf = (v: unknown): ShareKind | undefined => (v === 1 || v === 'SHARE_ITEM_KIND_MCP_SERVER' ? 'mcp_server' : v === 2 || v === 'SHARE_ITEM_KIND_SKILL' ? 'skill' : undefined);

/** The items of a `ShareItemListResult` (content asked for), by name; one the window cannot read is left out. */
export function sharedItemsOf(result: unknown): SharedItem[] {
  const items = obj(result).items;
  if (!Array.isArray(items)) return [];
  const out: SharedItem[] = [];
  for (const raw of items) {
    const i = obj(raw);
    const kind = kindOf(i.kind);
    if (!kind || typeof i.name !== 'string') continue;
    const scope = obj(i.scope);
    const m = obj(obj(obj(i.signed).item).mcp);
    const sk = obj(obj(obj(i.signed).item).skill);
    out.push({
      name: i.name, kind, hash: str(i.itemHash), issuedMs: num(i.issuedMs), signer: str(i.signerDeviceId),
      scope: { projectId: str(scope.projectId), kinds: Array.isArray(scope.kinds) ? scope.kinds.filter((k): k is string => typeof k === 'string') : [] },
      mcp: kind === 'mcp_server' && (m.command || m.url)
        ? { command: str(m.command), args: Array.isArray(m.args) ? m.args.map(str) : [], env: values(m.env), url: str(m.url), headers: values(m.headers) }
        : undefined,
      skill: kind === 'skill' && Array.isArray(sk.files)
        ? { description: str(sk.description), files: sk.files.map((f) => ({ path: str(obj(f).path), content: textOf(obj(f).content) })) }
        : undefined,
      refused: Array.isArray(i.refused) ? i.refused.map((x) => { const r = obj(x); return { node: str(r.nodeId), code: str(r.code), reason: str(r.reason) }; }) : [],
    });
  }
  return out.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

const failed = (e: unknown): SharedResult => {
  const err = e as { code?: unknown; params?: Record<string, unknown> };
  return { ok: false, code: typeof err?.code === 'string' ? err.code : 'unavailable', reason: typeof err?.params?.reason === 'string' ? err.params.reason : undefined };
};

/** The hub's Settings → Shared, through `ask` (the connection's hub-only commands), signing with `device`. A word's time is now, and
 *  always after the word it replaces (`after`), since a machine keeps only the newest word on a name. */
export function sharedSourceFor(ask: Ask, device: Device, now: () => number = Date.now): SharedSource {
  const sign = async (name: string, item: ShareItemDef | undefined, scope: ShareScopeDef, after = 0) =>
    signShareWord(device, name, item, scope, Math.max(now(), after + 1));
  return {
    async list() { return sharedItemsOf((await ask({ shareItemList: { withContent: true } })).shareItemList); },
    async publish(item, scope, after) {
      try { await ask({ shareItemPublish: { signed: await sign(item.name, item, scope, after) } }); return { ok: true }; } catch (e) { return failed(e); }
    },
    async remove(item) {
      try { await ask({ shareItemRemove: { signed: await sign(item.name, undefined, item.scope, item.issuedMs) } }); return { ok: true }; } catch (e) { return failed(e); }
    },
  };
}

/** The words of a machine's refusal (locale key and its arguments). */
export function refusalWords(r: SharedRefusal): { key: string; args?: Record<string, string> } {
  // The secret's name is the machine's word: shown with anything hidden in it made visible, and cut short.
  if (r.code === 'share.secret_missing') return { key: 'shared-refused-secret', args: { secret: visible(r.reason).slice(0, 64) } };
  const known = ['secret_needs_launcher', 'secret_not_allowed', 'env_not_allowed', 'policy_off', 'command_not_allowed', 'url_not_allowed', 'signer_untrusted', 'signature', 'older', 'invalid', 'skill_header', 'skill_not_written'];
  const reason = r.reason === 'signature' ? 'signer_untrusted' : r.reason;
  return { key: known.includes(reason) ? `shared-refused-${reason.replace(/_/g, '-')}` : 'shared-refused-other' };
}

/** The words of a publish or a removal the hub refused. */
export function publishErrorKey(r: { code: string; reason?: string }): string {
  if (r.code === 'approval.signature_invalid') return 'shared-error-device';
  if (r.code === 'share.refused' && (r.reason === 'older' || r.reason === 'limit')) return `shared-error-${r.reason}`;
  if (r.code === 'protocol.invalid_message' || r.code === 'share.refused') return 'shared-error-invalid';
  if (r.code === 'auth.forbidden') return 'shared-error-owner';
  return 'shared-error-other';
}

/** A name the rules allow: `[a-z0-9-]`, 1 to 64. */
export const validShareName = (s: string) => /^[a-z0-9-]{1,64}$/.test(s);
/** `NAME=value` lines (environment) or `Name: value` lines (headers) as values; `undefined` when a line has no separator or a name
 *  breaks the rules (`[A-Za-z0-9_-]`) or repeats. */
export function parseValues(text: string, sep: '=' | ':'): ShareValue[] | undefined {
  const out: ShareValue[] = [];
  for (const line of text.split('\n').map((l) => l.trim()).filter(Boolean)) {
    const at = line.indexOf(sep);
    if (at <= 0) return undefined;
    const name = line.slice(0, at).trim();
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(name) || out.some((v) => v.name === name)) return undefined;
    out.push({ name, value: line.slice(at + 1).trim() });
  }
  return out;
}
