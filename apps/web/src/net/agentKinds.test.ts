import { describe, expect, it } from 'vitest';
import { groupKinds, kindChoices, plainText, readKnown, runsAs, writeKnown, type KindInfo } from './agentKinds';

const kind = (over: Partial<KindInfo>): KindInfo => ({ kind: 'a', name: 'A', installed: true, level: 2, program: '', settings: [], options: [], ...over });

describe('agent kinds for Settings → Agents', () => {
  it('groups installed and not installed, by name, without the plain terminal or repeats', () => {
    const g = groupKinds([kind({ kind: 'z', name: 'Zed' }), kind({ kind: 'b', name: 'Beta', installed: false }), kind({ kind: 'shell', name: 'Terminal' }), kind({ kind: 'a', name: 'Alpha' }), kind({ kind: 'a', name: 'Again' })]);
    expect(g.installed.map((k) => k.kind)).toEqual(['a', 'z']);
    expect(g.missing.map((k) => k.kind)).toEqual(['b']);
  });
  it('says how an agent runs from its level', () => {
    expect([1, 2, 3, 4, 0].map(runsAs)).toEqual(['terminal', 'terminal', 'chat', 'adapter', 'terminal']);
  });
  it('shows third-party text as one short plain line', () => {
    expect(plainText('  a\n\tb‮evil\u0000 ')).toBe('a b evil');
    expect(plainText('Co\u034f\u2800\u3164\uffa0dex')).toBe('Co dex');
    expect(plainText('x'.repeat(100), 10)).toBe(`${'x'.repeat(9)}…`);
    expect(plainText('<img src=x onerror=alert(1)>')).toBe('<img src=x onerror=alert(1)>'); // kept as text; React never parses it
  });
  it('lists options that skip permissions last and flagged, drops unusable ids and never invents a default', () => {
    const c = kindChoices(kind({ options: [{ id: 'skip-permissions', label: 'x', skipsPermissions: true }, { id: 'fast', label: 'y', skipsPermissions: false }, { id: 'BAD ID', label: 'z', skipsPermissions: false }], settings: [{ id: 'model', label: 'setting-model', values: [], defaultValue: '' }, { id: 'Bad Id', label: '', values: [], defaultValue: '' }] }));
    expect(c.options.map((o) => [o.id, o.skipsPermissions])).toEqual([['fast', false], ['skip-permissions', true]]);
    expect(c.settings.map((s) => s.id)).toEqual(['model']);
  });
  it('keeps the last-known list per machine and drops what is malformed', () => {
    let saved = '';
    writeKnown({ setItem: (_k, v) => { saved = v; } }, new Map([['m1', { atMs: 5, kinds: [kind({ kind: 'a' })] }]]));
    const back = readKnown({ getItem: () => saved });
    expect(back.get('m1')?.atMs).toBe(5);
    expect(back.get('m1')?.kinds[0]?.kind).toBe('a');
    expect(readKnown({ getItem: () => '{"m":{"kinds":[1,{"kind":"a"}],"atMs":"x"},"n":{"kinds":[1,{"kind":"a","name":"A"}],"atMs":1}}' }).get('n')?.kinds).toHaveLength(1);
    expect(readKnown({ getItem: () => 'not json' }).size).toBe(0);
    expect(readKnown({ getItem: () => { throw new Error('blocked'); } }).size).toBe(0);
    expect(readKnown(undefined).size).toBe(0);
  });
});
