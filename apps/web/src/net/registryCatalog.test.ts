import { describe, expect, it } from 'vitest';
import catalog from '../../../../manifests/agents/registry/catalog.json';
import { groupKinds, type KindInfo } from './agentKinds';
import { cleanLine, isRegistryKind, registryCommit, registryCount, registryInfo, safeLink } from './registryCatalog';

describe('the ACP registry catalog (ADR-0092)', () => {
  it('knows every converted agent, each untested, with a version and an https link or none', () => {
    expect(registryCount()).toBe(catalog.agents.length);
    expect(registryCount()).toBeGreaterThanOrEqual(40);
    expect(registryCommit()).toMatch(/^[0-9a-f]{12}$/);
    for (const line of catalog.agents) {
      const info = registryInfo(line.kind)!;
      expect(info, line.kind).toBeDefined();
      expect(info.tested).toBe(false);
      expect(isRegistryKind(info.kind)).toBe(true);
      expect(info.version).not.toBe('');
      for (const href of [info.repository, info.website]) if (href) expect(href.startsWith('https://')).toBe(true);
    }
    // A hand-made kind is not in it.
    expect(registryInfo('claude-code')).toBeUndefined();
    expect(registryInfo('registry-goose')?.icon).toBeTruthy();
  });

  it('turns only https addresses into links', () => {
    expect(safeLink('https://github.com/x/y')).toBe('https://github.com/x/y');
    for (const bad of ['javascript:alert(1)', 'http://example.com', 'https://user:pw@example.com/', 'data:text/html,x', 'https://localhost/', '', 7, undefined, `https://e.com/${'a'.repeat(400)}`]) {
      expect(safeLink(bad), String(bad)).toBeUndefined();
    }
  });

  it('drops a line that is not one and cleans its text', () => {
    expect(cleanLine({ kind: 'registry-x', id: 'y', name: 'X', version: '1' })).toBeUndefined();
    expect(cleanLine({ kind: 'registry-../x', id: '../x', name: 'X', version: '1' })).toBeUndefined();
    expect(cleanLine(null)).toBeUndefined();
    const l = cleanLine({ kind: 'registry-x', id: 'x', name: 'Evil‮name', version: '1.0.0', runs: 'sh', repository: 'javascript:alert(1)', license: 'MIT' })!;
    expect(l.name).toBe('Evil name');
    expect(l.runs).toBe('binary');
    expect(l.repository).toBeUndefined();
    expect(l.license).toBe('MIT');
  });

  it('lists the hand-made kinds before the registry ones in each group', () => {
    const k = (kind: string, name: string, installed = true): KindInfo => ({ kind, name, installed, level: 3, program: '', settings: [], options: [] });
    const g = groupKinds([k('registry-aaa', 'Aaa'), k('zed', 'Zed'), k('registry-bbb', 'Bbb', false), k('opencode-acp', 'opencode', false)]);
    expect(g.installed.map((x) => x.kind)).toEqual(['zed', 'registry-aaa']);
    expect(g.missing.map((x) => x.kind)).toEqual(['opencode-acp', 'registry-bbb']);
  });
});
