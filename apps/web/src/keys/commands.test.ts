import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { isSafeKey } from '../plugins/safe';
import { COMMANDS, FIXED_KEYS, MESSAGE_ARGS, NEVER_COMMAND, fixedReason } from './commands';
import { type Binding, type Stroke, parseBinding } from './strokes';

const ids = (lang: string) => new Set([...readFileSync(new URL(`../../../../locales/${lang}/web.ftl`, import.meta.url), 'utf8').matchAll(/^([a-z][a-z0-9_-]*) =/gm)].map((m) => m[1] as string));

describe('the command list', () => {
  it('has one stable, well-formed id per command', () => {
    const all = COMMANDS.map((c) => c.id);
    expect(new Set(all).size).toBe(all.length);
    for (const id of all) expect(id).toMatch(/^[a-z][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*$/);
    for (const id of all) expect(NEVER_COMMAND.test(id), id).toBe(false);
  });
  it('gives every default a key string the file format accepts, never a fixed key (the safe-mode command\'s own excepted)', () => {
    for (const c of COMMANDS) {
      for (const k of c.keys) {
        const b = parseBinding(k) as Binding;
        expect(b, `${c.id}: ${k}`).toBeDefined();
        expect(b.some((s) => fixedReason(s) !== undefined), `${c.id}: ${k}`).toBe(!!c.fixed);
      }
    }
  });
  it('has the safe-mode key fixed, with Ctrl or ⌘, as plugins/safe.ts hears it', () => {
    const asEvent = (s: Stroke) => ({ code: `Key${s.key.toUpperCase()}`, ctrlKey: s.ctrl, metaKey: s.meta, altKey: s.alt, shiftKey: s.shift });
    const safe = FIXED_KEYS.filter((f) => f.reason === 'safe-mode').map((f) => (parseBinding(f.keys) as Binding)[0] as Stroke);
    expect(safe).toHaveLength(2);
    for (const s of safe) expect(isSafeKey(asEvent(s))).toBe(true);
    const cmd = COMMANDS.find((c) => c.id === 'window.safe-mode');
    expect(cmd?.fixed).toBe(true);
    expect(cmd?.keys.map((k) => isSafeKey(asEvent((parseBinding(k) as Binding)[0] as Stroke)))).toEqual([true, true]);
    // Only the safe-mode command is fixed.
    expect(COMMANDS.filter((c) => c.fixed).map((c) => c.id)).toEqual(['window.safe-mode']);
  });
  it('has words for every name, place and reason, in English and Turkish', () => {
    for (const lang of ['en', 'tr']) {
      const have = ids(lang);
      const need = new Set<string>();
      for (const c of COMMANDS) {
        need.add(c.label);
        need.add(`keys-context-${c.context}`);
        for (const [k, v] of Object.entries(c.args ?? {})) if (MESSAGE_ARGS.has(k) && typeof v === 'string') need.add(v);
      }
      for (const r of ['safe-mode', 'navigation', 'terminal', 'editing']) need.add(`keys-fixed-${r}`);
      for (const code of ['json', 'too-big', 'shape', 'newer', 'too-many', 'field', 'command', 'request', 'fixed-command', 'twice', 'too-many-keys', 'key', 'fixed-key', 'more']) need.add(`keys-error-${code}`);
      for (const k of need) expect(have.has(k), `${lang}: ${k}`).toBe(true);
    }
  });
});
