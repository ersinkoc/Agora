// Builds plugin packages for tests (never shipped: only test files import it).
import { createHash } from 'node:crypto';

export const sha = (s: string | Uint8Array) => createHash('sha256').update(s).digest('hex');
export const b64 = (s: string | Uint8Array) => Buffer.from(s).toString('base64');

export const ENTRY = "agora.watch('agents.read', function () {});";
export const baseManifest = (files: Record<string, string>, extra: Record<string, unknown> = {}) => ({
  id: 'demo', name: 'Demo', version: '1.0.0', author: 'Someone', licence: 'MIT', min_agora: '0.1.0',
  panels: [{ id: 'main', kind: 'global' }], capabilities: ['agents.read'], entry: 'panel.js',
  files: Object.fromEntries(Object.entries(files).map(([n, c]) => [n, sha(c)])),
  ...extra,
});

/** A package's bytes: the manifest (as given, or one listing `files`) and the files as base64. */
export function bundle(files: Record<string, string> = { 'panel.js': ENTRY }, opts: { manifest?: unknown; extraFiles?: Record<string, unknown>; top?: Record<string, unknown> } = {}): Uint8Array {
  const manifest = opts.manifest ?? baseManifest(files);
  const all: Record<string, unknown> = { 'agora-plugin.json': b64(JSON.stringify(manifest)), ...Object.fromEntries(Object.entries(files).map(([n, c]) => [n, b64(c)])), ...opts.extraFiles };
  return new TextEncoder().encode(JSON.stringify({ 'agora-plugin-bundle': 1, files: all, ...opts.top }));
}
