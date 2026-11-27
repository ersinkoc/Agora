import { describe, expect, it } from 'vitest';
import table from '../../../../manifests/agents/icons.json';
import { builtInIcons, kindIcon } from './kindIcons';
import { registryIcon, registryInfo } from './registryCatalog';

describe('kind marks', () => {
  it('gives the bundled kinds the registry icon of their tool', () => {
    expect(kindIcon('claude-code')).toBe(registryIcon('claude-acp'));
    expect(kindIcon('codex')).toBe(registryIcon('codex-acp'));
    expect(kindIcon('opencode-acp')).toBe(registryIcon('opencode'));
    expect(kindIcon('claude-code')).toBeTruthy();
  });

  it('keeps a registry kind on its own icon', () => {
    expect(kindIcon('registry-goose')).toBe(registryInfo('registry-goose')?.icon);
    expect(kindIcon('registry-goose')).toBeTruthy();
  });

  it('has no mark for a kind it does not know, so the avatar keeps its letter', () => {
    for (const k of ['shell', 'wrongstack', 'aider', '', undefined, 'Claude Code']) expect(kindIcon(k)).toBeUndefined();
  });

  it('names only icons the window has, and only plain ids', () => {
    const lines = Object.entries((table as { kinds: Record<string, string> }).kinds);
    expect(lines.length).toBeGreaterThan(0);
    for (const [kind, id] of lines) {
      expect(registryIcon(id), `${kind} → ${id}`).toBeTruthy();
      expect(kindIcon(kind)).toBe(registryIcon(id));
    }
    const bad = builtInIcons({ kinds: { 'Bad Kind': 'claude-acp', ok: '../x', gone: 'no-such-icon', fine: 'goose', n: 7 } });
    expect([...bad.keys()]).toEqual(['fine']);
    expect(builtInIcons(null).size).toBe(0);
    expect(builtInIcons({ kinds: [] }).size).toBe(0);
  });
});
