// A plugin package is checked whole against its own list (ADR-0114 decision 5 and Verification, "Packaging"): a path, a link, a file not
// in the list, a listed file missing, a wrong hash or over 2 MiB refuses the whole package.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { CAPABILITIES } from './capabilities';
import { MAX_PACKAGE_BYTES, checkPackage, compareVersions, isPlainName, typeOf } from './package';
import { packageErrorText } from './PluginsDialog';
import { ENTRY, b64, baseManifest, bundle } from './testPackage';
// @ts-expect-error: a plain script of the repository, without types
import { pack } from '../../../../scripts/pack-plugin.mjs';

const V = '0.3.39';
const code = async (bytes: Uint8Array) => { const r = await checkPackage(bytes, V); return r.ok ? 'ok' : r.code; };

describe('a plugin package', () => {
  it('passes with its manifest, its files and their hashes', async () => {
    const r = await checkPackage(bundle({ 'panel.js': ENTRY, 'panel.css': 'body{}' }), V);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.plugin.manifest).toMatchObject({ id: 'demo', name: 'Demo', version: '1.0.0', author: 'Someone', licence: 'MIT', minAgora: '0.1.0', entry: 'panel.js', capabilities: ['agents.read'] });
    expect([...r.plugin.files.keys()].sort()).toEqual(['panel.css', 'panel.js']);
    expect(new TextDecoder().decode(r.plugin.files.get('panel.js'))).toBe(ENTRY);
    // The manifest is not one of the plugin's files.
    expect(r.plugin.files.has('agora-plugin.json')).toBe(false);
  });

  it('refuses a package over 2 MiB before reading anything in it', async () => {
    const big = new Uint8Array(MAX_PACKAGE_BYTES + 1);
    expect(await code(big)).toBe('too-large');
    expect(await code(bundle({ 'panel.js': ENTRY, 'pad.txt': 'x'.repeat(MAX_PACKAGE_BYTES) }))).toBe('too-large');
  });

  it('refuses a name with a path in it, whatever the path', async () => {
    for (const name of ['../panel.js', 'a/panel.js', 'a\\panel.js', '/panel.js', 'C:panel.js', '.hidden.js', 'panel..js', '..', 'p%2fx.js', 'pa nel.js']) {
      expect(isPlainName(name), name).toBe(false);
      expect(await code(bundle({ 'panel.js': ENTRY }, { extraFiles: { [name]: b64('x') } })), name).toBe('path');
    }
    expect(isPlainName('panel.min.js')).toBe(true);
  });

  it('refuses an entry that is a link or anything but a file\'s bytes', async () => {
    for (const v of [{ link: '/etc/passwd' }, { type: 'symlink', target: '../x' }, ['x'], 42, null, true]) {
      expect(await code(bundle({ 'panel.js': ENTRY }, { extraFiles: { 'other.js': v } })), JSON.stringify(v)).toBe('link');
    }
  });

  it('refuses a file not in the list, a listed file missing, and a wrong hash, each refusing the whole package', async () => {
    expect(await code(bundle({ 'panel.js': ENTRY }, { extraFiles: { 'extra.js': b64('alert(1)') } }))).toBe('unlisted');
    expect(await code(bundle({ 'panel.js': ENTRY }, { manifest: baseManifest({ 'panel.js': ENTRY, 'gone.css': 'x' }) }))).toBe('missing');
    const wrong = baseManifest({ 'panel.js': ENTRY, 'panel.css': 'body{}' });
    (wrong.files as Record<string, string>)['panel.css'] = '0'.repeat(64);
    const r = await checkPackage(bundle({ 'panel.js': ENTRY, 'panel.css': 'body{}' }, { manifest: wrong }), V);
    expect(r).toEqual({ ok: false, code: 'hash', detail: 'panel.css' });
  });

  it('refuses what is not a package, a newer bundle format, extra fields and a kind of file a plugin cannot use', async () => {
    expect(await code(new TextEncoder().encode('PK\u0003\u0004 not json'))).toBe('format');
    expect(await code(new TextEncoder().encode(JSON.stringify({ 'agora-plugin-bundle': 2, files: {} })))).toBe('newer-format');
    expect(await code(bundle(undefined, { top: { run: 'x' } }))).toBe('format');
    expect(await code(bundle({ 'panel.js': ENTRY, 'tool.exe': 'MZ' }))).toBe('type');
    // A name whose extension is a property every object inherits is no kind of file either.
    for (const name of ['a.constructor', 'a.__proto__', 'a.toString', 'a.hasOwnProperty', 'a.valueOf']) {
      expect(typeOf(name), name).toBeUndefined();
      expect(await code(bundle({ 'panel.js': ENTRY }, { extraFiles: { [name]: b64('x') } })), name).toMatch(/^(type|path)$/);
    }
    expect(await code(bundle({ 'panel.js': ENTRY }, { extraFiles: { 'agora-plugin.json': 'not base64!' } }))).toBe('format');
  });

  it('checks the manifest field by field and refuses unknown fields and capabilities', async () => {
    const files = { 'panel.js': ENTRY };
    const with_ = (extra: Record<string, unknown>) => bundle(files, { manifest: baseManifest(files, extra) });
    expect(await code(with_({ network: ['example.com'] }))).toBe('manifest');
    expect(await code(with_({ capabilities: ['agents.read', 'terminal.read'] }))).toBe('capability');
    expect(await code(with_({ capabilities: ['actions.prompt'] }))).toBe('capability');
    expect(await code(with_({ entry: 'panel.css' }))).toBe('manifest');
    expect(await code(with_({ id: 'Bad Id' }))).toBe('manifest');
    expect(await code(with_({ name: 'Needs‮you' }))).toBe('manifest');
    expect(await code(with_({ name: '' }))).toBe('manifest');
    expect(await code(with_({ panels: [{ id: 'main', kind: 'chooser' }] }))).toBe('manifest');
    expect(await code(with_({ panels: [{ id: 'main', kind: 'global', lone: true }] }))).toBe('manifest');
    expect(await code(with_({ panels: [{ id: 'main', streams: true }] }))).toBe('manifest');
    expect(await code(with_({ panels: [] }))).toBe('manifest');
    expect(await code(with_({ version: '1.0' }))).toBe('manifest');
    // Every rung-1 capability is accepted.
    expect(await code(with_({ capabilities: [...CAPABILITIES] }))).toBe('ok');
  });

  it('refuses a plugin that needs a newer window', async () => {
    const files = { 'panel.js': ENTRY };
    expect(await checkPackage(bundle(files, { manifest: baseManifest(files, { min_agora: '9.0.0' }) }), V)).toEqual({ ok: false, code: 'newer', detail: '9.0.0' });
    expect(compareVersions('0.3.39', '0.3.4')).toBe(1);
    expect(compareVersions('0.10.0', '0.9.9')).toBe(1);
    expect(compareVersions('1.0.0', '1.0.0')).toBe(0);
  });

  it('words every refusal in English and Turkish, with the file it is about', () => {
    for (const lang of ['en', 'tr']) {
      setLanguage(lang);
      for (const c of ['too-large', 'format', 'newer-format', 'too-many', 'path', 'link', 'type', 'manifest', 'capability', 'unlisted', 'missing', 'hash', 'newer', 'full'] as const) {
        expect(packageErrorText(c, 'x.js'), c).not.toMatch(/plugin-error/);
      }
      expect(packageErrorText('hash', 'panel.css')).toContain('panel.css');
      // A name with invisible characters is shown without them.
      expect(packageErrorText('path', 'a‮/b')).not.toContain('‮');
    }
    setLanguage('en');
  });
});

describe('the sample plugin (apps/web/plugins/sample)', () => {
  it('packs from its folder with the hashes its manifest lists, and passes the window\'s checks', async () => {
    const text: string = pack(join(__dirname, '../../plugins/sample'));
    const r = await checkPackage(new TextEncoder().encode(text), V);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.plugin.manifest.capabilities).toEqual(['agents.read', 'theme.read']);
    const cargo = /^version = "([^"]+)"/m.exec(readFileSync(join(__dirname, '../../../../Cargo.toml'), 'utf8'))?.[1] ?? '';
    expect(compareVersions(r.plugin.manifest.minAgora, cargo)).toBeLessThanOrEqual(0);
  });
});

// ADR-0114 decision 6: the hub checks a package exactly as the window does (crates/agora-hub/src/plugin_package.rs). Both read the same
// packages and the same expectations (crates/agora-hub/tests/fixtures/plugin-packages/expected.json; the Rust side is
// tests/plugin_package_agreement.rs): the same packages pass, and the others are refused with the same code and detail.
describe('the shared packages the hub checks too', () => {
  const dir = join(__dirname, '../../../../crates/agora-hub/tests/fixtures/plugin-packages');
  const expected = JSON.parse(readFileSync(join(dir, 'expected.json'), 'utf8')) as {
    agoraVersion: string;
    cases: { file: string; ok: boolean; id?: string; version?: string; capabilities?: string[]; code?: string; detail?: string }[];
  };
  it('has every case', () => { expect(expected.cases.length).toBeGreaterThanOrEqual(30); });
  for (const c of expected.cases) {
    it(`${c.file}: ${c.ok ? 'passes' : `refused, ${c.code}`}`, async () => {
      const r = await checkPackage(new Uint8Array(readFileSync(join(dir, c.file))), expected.agoraVersion);
      if (c.ok) {
        expect(r.ok).toBe(true);
        if (!r.ok) return;
        expect({ id: r.plugin.manifest.id, version: r.plugin.manifest.version, capabilities: [...r.plugin.manifest.capabilities] })
          .toEqual({ id: c.id, version: c.version, capabilities: c.capabilities });
      } else {
        expect(r.ok).toBe(false);
        if (r.ok) return;
        expect({ code: r.code, detail: r.detail }).toEqual({ code: c.code, detail: c.detail });
      }
    });
  }
});
