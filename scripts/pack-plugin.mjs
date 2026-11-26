// Packs a plugin folder into a package (ADR-0114 decision 5; docs/ux/plugins.md, "The package"): one JSON bundle with
// `agora-plugin.json` and every file it lists, each as base64. The folder is flat (no sub-folders); every file in it but the manifest
// must be listed with its SHA-256, and a wrong or missing hash stops the pack, as the window would refuse the package. `README.md` and
// `agora-plugin.d.ts` (the types `agora plugin new` puts in a folder) are never packed. A link in the folder stops the pack: nothing
// outside the folder is read.
//
// `agora plugin pack <folder> [--out <file>] [--write-hashes]` makes the same package byte for byte (crates/agora/src/pluginkit.rs; both
// are held to the same packages in crates/agora/tests/fixtures/plugin-kit/), and also runs the window's checks on it.
//
//   node scripts/pack-plugin.mjs apps/web/plugins/sample                     # the package on stdout
//   node scripts/pack-plugin.mjs apps/web/plugins/sample --out sample.json   # into a file
//   node scripts/pack-plugin.mjs apps/web/plugins/sample --write-hashes      # write the files' hashes into the manifest first
import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export const MANIFEST = 'agora-plugin.json';
/** Files of a plugin's folder that are never packed: its README and the types for the author's editor. */
export const NOT_PACKED = ['README.md', 'agora-plugin.d.ts'];
const sha256 = (b) => createHash('sha256').update(b).digest('hex');

/** The package text of a folder; throws with a message saying what is wrong. */
export function pack(dir, { writeHashes = false } = {}) {
  const names = readdirSync(dir).filter((n) => !n.startsWith('.')).sort();
  for (const n of names) if (!lstatSync(join(dir, n)).isFile()) throw new Error(`${n}: not a file (a plugin folder is flat)`);
  if (!names.includes(MANIFEST)) throw new Error(`${MANIFEST} is missing`);
  const manifest = JSON.parse(readFileSync(join(dir, MANIFEST), 'utf8'));
  const others = names.filter((n) => n !== MANIFEST && !NOT_PACKED.includes(n));
  if (writeHashes) {
    manifest.files = Object.fromEntries(others.map((n) => [n, sha256(readFileSync(join(dir, n)))]));
    writeFileSync(join(dir, MANIFEST), `${JSON.stringify(manifest, null, 2)}\n`);
  }
  const listed = Object.keys(manifest.files ?? {});
  for (const n of others) if (!listed.includes(n)) throw new Error(`${n}: not listed in ${MANIFEST}`);
  for (const n of listed) {
    if (!others.includes(n)) throw new Error(`${n}: listed in ${MANIFEST} but missing`);
    if (sha256(readFileSync(join(dir, n))) !== manifest.files[n]) throw new Error(`${n}: its hash in ${MANIFEST} is not the file's (run with --write-hashes)`);
  }
  const files = Object.fromEntries([MANIFEST, ...listed].map((n) => [n, readFileSync(join(dir, n)).toString('base64')]));
  return JSON.stringify({ 'agora-plugin-bundle': 1, files });
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [dir, ...rest] = process.argv.slice(2);
  if (!dir) { console.error('usage: node scripts/pack-plugin.mjs <folder> [--out <file>] [--write-hashes]'); process.exit(2); }
  try {
    const text = pack(dir, { writeHashes: rest.includes('--write-hashes') });
    const out = rest.includes('--out') ? rest[rest.indexOf('--out') + 1] : undefined;
    if (out) writeFileSync(out, text); else process.stdout.write(text);
  } catch (e) {
    console.error(`pack-plugin: ${e.message}`);
    process.exit(1);
  }
}
