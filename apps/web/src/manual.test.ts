// `agora manual` (G10, docs/manual/): the examples and lists of the topics about the web window's files, read by the window's own
// readers. The Rust side (command lines, the terminal client's readers, mesh tools, file names) is tested in
// crates/agora/src/manualcmd.rs.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { COMMANDS } from './keys/commands';
import { MAX_ENTRIES, MAX_FILE_BYTES as MAX_KEYS_BYTES, MAX_KEYS_PER_COMMAND, readKeys } from './keys/file';
import { BUILTIN_PANELS } from './layout/panels';
import { MAX_DEPTH, MAX_PANES, deserialiseLayout } from './layout/tree';
import { CAPABILITIES } from './plugins/capabilities';
import { MAX_FILES, MAX_PACKAGE_BYTES, checkPackage, typeOf } from './plugins/package';
import { MAX_DEV_PLUGINS } from './plugins/store';
import { MAX_FILE_BYTES as MAX_THEME_BYTES, deserialise } from './themes/file';
// @ts-expect-error: a plain script of the repository, without types
import { pack } from '../../../scripts/pack-plugin.mjs';

const topic = (id: string) => readFileSync(new URL(`../../../docs/manual/${id}.md`, import.meta.url), 'utf8');

/** The fenced blocks of a topic, as the Rust test reads them: the info string, and the lines between the fences with a final newline. */
function blocks(text: string): { info: string; body: string }[] {
  const out: { info: string; body: string }[] = [];
  let open: { info: string; lines: string[] } | undefined;
  for (const line of text.split('\n')) {
    if (!open && line.startsWith('```')) open = { info: line.slice(3).trim(), lines: [] };
    else if (open && line === '```') { out.push({ info: open.info, body: `${open.lines.join('\n')}\n` }); open = undefined; }
    else if (open) open.lines.push(line);
  }
  expect(open).toBeUndefined();
  return out;
}
const block = (id: string, info: string) => {
  const found = blocks(topic(id)).filter((b) => b.info === info);
  expect(found).toHaveLength(1);
  return found[0]?.body ?? '';
};
/** The first column's code of a table that starts with this header. */
function tableIds(text: string, header: string): string[] {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => l.startsWith(header));
  expect(start).toBeGreaterThanOrEqual(0);
  const rows: string[] = [];
  for (const l of lines.slice(start + 2)) {
    if (!l.startsWith('| ')) break;
    rows.push(l.split('|')[1] ?? '');
  }
  return rows;
}
const codes = (cell: string) => [...cell.matchAll(/`([^`]+)`/g)].map((m) => m[1] ?? '');

describe('agora manual themes', () => {
  it('has examples the web window accepts without a warning', () => {
    const themes = blocks(topic('themes')).filter((b) => b.info.startsWith('json') && b.body.includes('"agora-theme"'));
    expect(themes).toHaveLength(2);
    for (const t of themes) {
      const r = deserialise(t.body);
      expect(r.ok ? [] : r.issues).toEqual([]);
      expect(r.ok && (r.warnings ?? [])).toEqual([]);
    }
  });
  it('states the size cap', () => {
    expect(topic('themes')).toContain(`larger than ${MAX_THEME_BYTES / 1024} KiB`);
  });
});

describe('agora manual layouts', () => {
  it('has an example the web window reads as it is written', () => {
    const text = block('layouts', 'json agora-layout');
    const r = deserialiseLayout(text);
    expect(r.ok).toBe(true);
    expect(r.ok && r.layout).toEqual(JSON.parse(text));
  });
  it('lists every panel of the window, and its limits', () => {
    const ids = tableIds(topic('layouts'), '| Panel |').flatMap(codes);
    expect([...ids].sort()).toEqual(BUILTIN_PANELS.map((p) => p.id).sort());
    expect(topic('layouts')).toContain(`At most ${MAX_PANES} panes`);
    expect(topic('layouts')).toContain(`at most ${MAX_DEPTH} levels`);
  });
});

describe('agora manual keys', () => {
  it('has files the web window reads, with no command it does not know', () => {
    const files = blocks(topic('keys')).filter((b) => b.info.startsWith('json'));
    expect(files).toHaveLength(2);
    for (const f of files) {
      const r = readKeys(f.body);
      expect(r.ok ? [] : r.issues).toEqual([]);
      expect(r.ok && r.unknown).toEqual([]);
    }
    expect(files.map((f) => f.info)).toContain('json agora-keys');
  });
  it('lists every command, and only real ones', () => {
    const named = new Set<string>();
    for (const cell of tableIds(topic('keys'), '| Command |')) {
      // `pane.tab-1` … `pane.tab-9` names the nine.
      const range = /^\s*`([a-z.-]+-)(\d)` … `\1(\d)`/.exec(cell);
      if (range) for (let i = Number(range[2]); i <= Number(range[3]); i++) named.add(`${range[1] ?? ''}${i}`);
      else for (const c of codes(cell)) named.add(c);
    }
    expect([...named].sort()).toEqual(COMMANDS.map((c) => c.id).sort());
  });
  it('states the limits', () => {
    const s = topic('keys');
    expect(s).toContain(`at most ${MAX_KEYS_BYTES / 1024} KiB`);
    expect(s).toContain(`at most ${MAX_ENTRIES} entries`);
    expect(s).toContain(`At most ${MAX_KEYS_PER_COMMAND} keys per command`);
  });
});

describe('agora manual plugins', () => {
  it('has an example package the window accepts, packed by the repository script too', async () => {
    const manifest = block('plugins', 'json agora-plugin');
    const panel = block('plugins', 'js panel.js');
    const dir = mkdtempSync(join(tmpdir(), 'agora-manual-'));
    try {
      writeFileSync(join(dir, 'agora-plugin.json'), manifest);
      writeFileSync(join(dir, 'panel.js'), panel);
      // The topic's own recipe: the manifest and the files, each as base64.
      const byHand = JSON.stringify({ 'agora-plugin-bundle': 1, files: { 'agora-plugin.json': Buffer.from(manifest).toString('base64'), 'panel.js': Buffer.from(panel).toString('base64') } });
      for (const text of [byHand, pack(dir) as string]) {
        const r = await checkPackage(new TextEncoder().encode(text), __AGORA_VERSION__);
        expect(r.ok ? 'ok' : `${r.code} ${r.detail ?? ''}`).toBe('ok');
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
  it('lists every capability, and the limits', () => {
    const s = topic('plugins');
    expect(tableIds(s, '| Capability |').flatMap(codes).sort()).toEqual([...CAPABILITIES].sort());
    expect(s).toContain(`at most ${MAX_PACKAGE_BYTES / 1024 / 1024} MiB`);
    expect(s).toContain(`At most ${MAX_FILES} files`);
    expect(s).toContain(`at most ${MAX_DEV_PLUGINS} are loaded`);
    const kinds = /Allowed kinds, by extension: (.*)\./.exec(s)?.[1] ?? '';
    for (const ext of codes(kinds)) expect(typeOf(`a.${ext}`), ext).toBeDefined();
    expect(typeOf('a.html')).toBeUndefined();
    expect(s).not.toContain('`html`');
  });
});
