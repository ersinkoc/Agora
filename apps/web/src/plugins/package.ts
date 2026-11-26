// A plugin package (ADR-0114 decision 5; docs/ux/plugins.md, "The package"). A package is one JSON bundle of at most 2 MiB:
//
//   { "agora-plugin-bundle": 1, "files": { "agora-plugin.json": "<base64>", "panel.js": "<base64>", … } }
//
// A JSON bundle rather than a zip: reading a zip's deflated entries would need a new dependency, and a flat map of names to bytes has no
// directories, links or extra fields to get wrong (the Zed unpacking CVEs were in exactly those). `agora-plugin.json` is the manifest; it
// lists every other file with its SHA-256. The package is unpacked in memory and checked against that list: a name with a path in it, an
// entry that is not a file, a file not listed, a listed file missing or a wrong hash refuses the whole package. Nothing of a refused
// package is kept or run. Reading never throws: every outcome is a result with a stable code (the window words it, `plugin-error-*`).
import { type Capability, isCapability } from './capabilities';

export const MAX_PACKAGE_BYTES = 2 * 1024 * 1024;
export const MAX_FILES = 64;
export const MANIFEST = 'agora-plugin.json';
export const BUNDLE_VERSION = 1;

/** A panel a plugin adds. `kind` is one of the two a plugin may have (ADR-0114 decision 4; layout.md, "Host-set facts"). */
export interface PluginPanelDecl { readonly id: string; readonly kind: 'global' | 'tool'; readonly name?: string }
export interface PluginManifest {
  readonly id: string;
  readonly name: string;
  readonly version: string;
  readonly author: string;
  readonly licence: string;
  readonly minAgora: string;
  readonly panels: readonly PluginPanelDecl[];
  readonly capabilities: readonly Capability[];
  readonly entry: string;
  /** Every file but the manifest, with its SHA-256 in lowercase hex. */
  readonly files: Readonly<Record<string, string>>;
}
export interface CheckedPlugin {
  readonly manifest: PluginManifest;
  readonly files: ReadonlyMap<string, Uint8Array>;
}

export type PackageError =
  | 'too-large' | 'format' | 'newer-format' | 'too-many' | 'path' | 'link' | 'type' | 'manifest' | 'capability' | 'unlisted' | 'missing'
  | 'hash' | 'newer';
export type PackageResult = { ok: true; plugin: CheckedPlugin } | { ok: false; code: PackageError; detail?: string };

// What a file may be, by its extension: what the frame can use (a script, a stylesheet, an image, a font) and plain data.
const TYPES: Record<string, string> = {
  js: 'text/javascript', css: 'text/css', json: 'application/json', txt: 'text/plain',
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml',
  woff: 'font/woff', woff2: 'font/woff2',
};
export const typeOf = (name: string): string | undefined => {
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return undefined;
  const ext = name.slice(dot + 1).toLowerCase();
  // The table's own entries only: `a.constructor` or `a.__proto__` names nothing a plugin may hold.
  return Object.prototype.hasOwnProperty.call(TYPES, ext) ? TYPES[ext] : undefined;
};

// A file name and nothing more: no folder, no "..", no drive, no leading dot, nothing a path could hide behind.
const NAME = /^[A-Za-z0-9][A-Za-z0-9_-]*(\.[A-Za-z0-9_-]+)*$/;
export const isPlainName = (s: string) => s.length <= 100 && NAME.test(s);
const ID = /^[a-z0-9][a-z0-9-]{0,39}$/;
const PANEL_ID = /^[a-z0-9][a-z0-9-]{0,19}$/;
const VERSION = /^(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})$/;
const HASH = /^[0-9a-f]{64}$/;
// Text the window shows as it is (a name, an author): one line, no control or invisible characters (bidi marks included), so it cannot
// pose as two words of the window's own or reorder what stands beside it.
// eslint-disable-next-line no-control-regex
const UNSAFE_TEXT = /[\u0000-\u001f\u007f-\u009f­؜᠎​-‏‪-‮⁠-⁯﻿￹-￻]/;
const plainText = (v: unknown, max: number): v is string => typeof v === 'string' && v.trim() === v && v.length > 0 && v.length <= max && !UNSAFE_TEXT.test(v);

const MANIFEST_KEYS = ['id', 'name', 'version', 'author', 'licence', 'min_agora', 'panels', 'capabilities', 'entry', 'files'];
const PANEL_KEYS = ['id', 'kind', 'name'];

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const own = (o: object, k: string) => Object.prototype.hasOwnProperty.call(o, k);

export function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) { const d = (pa[i] ?? 0) - (pb[i] ?? 0); if (d !== 0) return d < 0 ? -1 : 1; }
  return 0;
}

function base64(s: string): Uint8Array | undefined {
  if (s.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(s)) return undefined;
  try {
    const bin = atob(s);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch { return undefined; }
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const d = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as Uint8Array<ArrayBuffer>));
  return [...d].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** The manifest as a plugin wrote it, checked field by field. Unknown fields are refused: a newer manifest may ask for something this
 *  window does not know how to keep a plugin out of. */
export function readManifest(v: unknown): { ok: true; manifest: PluginManifest } | { ok: false; code: 'manifest' | 'capability'; detail: string } {
  const bad = (detail: string) => ({ ok: false as const, code: 'manifest' as const, detail });
  if (!isObject(v)) return bad('manifest');
  for (const k of Object.keys(v)) if (!MANIFEST_KEYS.includes(k)) return bad(k);
  if (typeof v.id !== 'string' || !ID.test(v.id)) return bad('id');
  if (!plainText(v.name, 60)) return bad('name');
  if (typeof v.version !== 'string' || !VERSION.test(v.version)) return bad('version');
  if (!plainText(v.author, 100)) return bad('author');
  if (!plainText(v.licence, 60)) return bad('licence');
  if (typeof v.min_agora !== 'string' || !VERSION.test(v.min_agora)) return bad('min_agora');
  if (!Array.isArray(v.panels) || v.panels.length < 1 || v.panels.length > 4) return bad('panels');
  const panels: PluginPanelDecl[] = [];
  for (const p of v.panels) {
    if (!isObject(p) || Object.keys(p).some((k) => !PANEL_KEYS.includes(k))) return bad('panels');
    if (typeof p.id !== 'string' || !PANEL_ID.test(p.id) || panels.some((x) => x.id === p.id)) return bad('panels');
    const kind = p.kind === undefined ? 'global' : p.kind;
    if (kind !== 'global' && kind !== 'tool') return bad('panels');
    if (p.name !== undefined && !plainText(p.name, 60)) return bad('panels');
    panels.push(Object.freeze({ id: p.id, kind, ...(p.name !== undefined ? { name: p.name as string } : {}) }));
  }
  if (!Array.isArray(v.capabilities) || v.capabilities.length > 16) return bad('capabilities');
  const caps: Capability[] = [];
  for (const c of v.capabilities) {
    if (!isCapability(c)) return { ok: false, code: 'capability', detail: typeof c === 'string' ? c.slice(0, 40) : '?' };
    if (!caps.includes(c)) caps.push(c);
  }
  if (!isObject(v.files)) return bad('files');
  const files: Record<string, string> = Object.create(null);
  for (const [name, hash] of Object.entries(v.files)) {
    if (!isPlainName(name) || name === MANIFEST || typeof hash !== 'string' || !HASH.test(hash)) return bad('files');
    files[name] = hash;
  }
  if (typeof v.entry !== 'string' || !own(files, v.entry) || typeOf(v.entry) !== 'text/javascript') return bad('entry');
  return {
    ok: true,
    manifest: Object.freeze({
      id: v.id, name: v.name, version: v.version, author: v.author, licence: v.licence, minAgora: v.min_agora,
      panels: Object.freeze(panels), capabilities: Object.freeze(caps), entry: v.entry, files: Object.freeze(files),
    }),
  };
}

/** Checks a package's bytes against its own list and this window's version. Nothing is kept from a package that fails. */
export async function checkPackage(bytes: Uint8Array, agoraVersion: string): Promise<PackageResult> {
  if (bytes.length > MAX_PACKAGE_BYTES) return { ok: false, code: 'too-large' };
  let top: unknown;
  try { top = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); } catch { return { ok: false, code: 'format' }; }
  if (!isObject(top) || !own(top, 'agora-plugin-bundle') || !own(top, 'files') || Object.keys(top).length !== 2) return { ok: false, code: 'format' };
  const ver = top['agora-plugin-bundle'];
  if (typeof ver === 'number' && Number.isInteger(ver) && ver > BUNDLE_VERSION) return { ok: false, code: 'newer-format' };
  if (ver !== BUNDLE_VERSION || !isObject(top.files)) return { ok: false, code: 'format' };
  const entries = Object.entries(top.files);
  if (entries.length > MAX_FILES + 1) return { ok: false, code: 'too-many' };
  const files = new Map<string, Uint8Array>();
  for (const [name, value] of entries) {
    if (!isPlainName(name)) return { ok: false, code: 'path', detail: name.slice(0, 100) };
    // Anything but the bytes of a file (a link, a folder, an object with a target) is refused, whatever it says it is.
    if (typeof value !== 'string') return { ok: false, code: 'link', detail: name };
    if (name !== MANIFEST && !typeOf(name)) return { ok: false, code: 'type', detail: name };
    const b = base64(value);
    if (!b) return { ok: false, code: 'format', detail: name };
    files.set(name, b);
  }
  const raw = files.get(MANIFEST);
  if (!raw) return { ok: false, code: 'manifest', detail: MANIFEST };
  let json: unknown;
  try { json = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(raw)); } catch { return { ok: false, code: 'manifest', detail: MANIFEST }; }
  const m = readManifest(json);
  if (!m.ok) return m;
  files.delete(MANIFEST);
  for (const name of files.keys()) if (!own(m.manifest.files, name)) return { ok: false, code: 'unlisted', detail: name };
  for (const name of Object.keys(m.manifest.files)) if (!files.has(name)) return { ok: false, code: 'missing', detail: name };
  for (const [name, b] of files) if ((await sha256Hex(b)) !== m.manifest.files[name]) return { ok: false, code: 'hash', detail: name };
  if (compareVersions(m.manifest.minAgora, agoraVersion) > 0) return { ok: false, code: 'newer', detail: m.manifest.minAgora };
  return { ok: true, plugin: Object.freeze({ manifest: m.manifest, files }) };
}
