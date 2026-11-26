// The plugin author kit (roadmap M5.7): `scripts/pack-plugin.mjs` and `agora plugin pack` make the same package byte for byte (both are
// held to the packages in crates/agora/tests/fixtures/plugin-kit/, made by the mjs; the Rust side is crates/agora/tests/plugin_kit.rs),
// the starter `agora plugin new` copies passes the window's checks, and the author's types name exactly what the loader gives a plugin
// (the types themselves are checked against the window's by `tsc -p plugins`, plugins/kit.check.ts).
import { cpSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { checkPackage } from './package';
// @ts-expect-error: a plain script of the repository, without types
import { NOT_PACKED, pack } from '../../../../scripts/pack-plugin.mjs';

const repo = join(__dirname, '../../../..');
const fixtures = join(repo, 'crates/agora/tests/fixtures/plugin-kit');
const starter = join(repo, 'apps/web/plugins/starter');
const V = '0.3.39';

describe('pack-plugin.mjs and agora plugin pack', () => {
  it('make the packages both are held to', () => {
    expect(pack(join(fixtures, 'listed-order'))).toBe(readFileSync(join(fixtures, 'listed-order.bundle'), 'utf8'));
    const dir = mkdtempSync(join(tmpdir(), 'agora-kit-'));
    try {
      cpSync(join(fixtures, 'write-hashes'), dir, { recursive: true });
      expect(pack(dir, { writeHashes: true })).toBe(readFileSync(join(fixtures, 'write-hashes.bundle'), 'utf8'));
      expect(readFileSync(join(dir, 'agora-plugin.json'))).toEqual(readFileSync(join(fixtures, 'write-hashes.manifest')));
    } finally { rmSync(dir, { recursive: true, force: true }); }
  });

  it('leave out the same files', () => {
    const rust = readFileSync(join(repo, 'crates/agora/src/pluginkit.rs'), 'utf8');
    const m = /const NOT_PACKED: \[&str; \d+\] = \[([^\]]*)\]/.exec(rust);
    expect(m && [...m[1]!.matchAll(/"([^"]+)"/g)].map((x) => x[1])).toEqual(NOT_PACKED);
  });
});

describe('the starter agora plugin new copies', () => {
  it('packs as it is and passes the window\'s checks, without its README and types', async () => {
    const text: string = pack(starter);
    const r = await checkPackage(new TextEncoder().encode(text), V);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect([...r.plugin.files.keys()].sort()).toEqual(['panel.css', 'panel.js']);
    expect(readFileSync(join(starter, 'panel.js'), 'utf8')).toContain('/// <reference path="agora-plugin.d.ts" />');
  });
});

describe('the author\'s types (agora-plugin.d.ts)', () => {
  const dts = readFileSync(join(starter, 'agora-plugin.d.ts'), 'utf8');
  const loader = readFileSync(join(repo, 'apps/web/public/plugin-frame.js'), 'utf8');
  const block = (src: string, start: string) => {
    const at = src.indexOf(start);
    expect(at, start).toBeGreaterThan(-1);
    let depth = 0;
    for (let i = src.indexOf('{', at); i < src.length; i++) {
      if (src[i] === '{') depth++;
      else if (src[i] === '}' && --depth === 0) return src.slice(src.indexOf('{', at) + 1, i);
    }
    return '';
  };
  /** The names of an object's or an interface's own members (nested blocks left out). */
  const members = (body: string) => {
    let flat = '';
    let depth = 0;
    for (const c of body) { if (c === '{' || c === '(') depth++; if (depth === 0) flat += c; if (c === '}' || c === ')') depth--; }
    return [...flat.matchAll(/(?:^|[;,\n])\s*(?:\/\*\*[^]*?\*\/\s*)?(?:readonly\s+)?([A-Za-z]+)\s*[:<(]/g)].map((x) => x[1]).sort();
  };

  it('name every member of the `agora` the loader makes, and nothing else', () => {
    const api = members(block(loader, 'var api = {'));
    expect(api).toEqual(['file', 'granted', 'panel', 'plugin', 'read', 'scheme', 'version', 'watch']);
    expect(members(block(dts, 'interface Api {'))).toEqual(api);
  });

  it('name every message the loader sends and takes', () => {
    const sent = new Set([...loader.matchAll(/send\(\{ type: '([a-z]+)'/g)].map((x) => x[1]));
    const taken = new Set([...loader.matchAll(/m\.type === '([a-z]+)'/g)].map((x) => x[1]));
    const types = (from: string, to?: string) => new Set([...dts.slice(dts.indexOf(from), to ? dts.indexOf(to) : undefined).matchAll(/type: '([a-z]+)'/g)].map((x) => x[1]));
    expect(sent).toEqual(new Set(['boot', 'read', 'watch', 'unwatch']));
    expect(types('type FrameMessage =', 'type HostMessage =')).toEqual(sent);
    expect(types('type HostMessage =')).toEqual(taken);
  });
});
