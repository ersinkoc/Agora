// The keybindings file (docs/ux/keybindings.md): the person's own keys for commands, over the defaults. The one door from untrusted text
// (an imported file, this browser's storage) into keys. Like a theme file it is data and never run: it is read with JSON.parse only, its
// fields are a closed set, a malformed key string or a key the window keeps fixed refuses the whole file, and every problem is reported
// at once. A command this window does not know is kept (so a file written by a newer window survives a round trip) and marked.
import { COMMANDS, type CommandDef, type FixedReason, NEVER_COMMAND, commandById, fixedReason } from './commands';
import { type Binding, bindingText, parseBinding, startsWith } from './strokes';

export const KEYS_FORMAT = 1;
export const MAX_FILE_BYTES = 64 * 1024;
export const MAX_ENTRIES = 200;
export const MAX_KEYS_PER_COMMAND = 4;
export const MAX_ISSUES = 50;

/** One command's keys as the file gives them; `keys: []` takes every key from it. */
export interface KeyEntry { command: string; keys: readonly string[] }
export interface KeyFile { entries: readonly KeyEntry[] }

export type KeyIssue =
  | { code: 'json' | 'too-big' | 'shape' | 'newer' | 'too-many' }
  | { code: 'field'; at: string }
  | { code: 'command' | 'request' | 'fixed-command' | 'twice' | 'too-many-keys'; command: string }
  | { code: 'key'; command: string; key: string }
  | { code: 'fixed-key'; command: string; key: string; reason: FixedReason }
  | { code: 'more'; count: number };

/** `unknown`: the commands the file names that this window does not have, kept and not used. */
export type ReadKeys = { ok: true; file: KeyFile; unknown: string[] } | { ok: false; issues: KeyIssue[] };

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const own = (o: object, k: string) => Object.prototype.hasOwnProperty.call(o, k);
// Text from the file shown back in a problem: no control or direction characters, and short.
const shown = (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v) ?? '').replace(/[\u0000-\u001f\u007f-\u009f‎‏‪-‮⁦-⁩]/g, '').slice(0, 64);
// A command id: dotted lower-case words (`layout.preset-2x2`). Anything else is not an id, rather than an unknown one.
const ID = /^[a-z][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*$/;
const TOP = new Set(['version', 'bindings']);
const ENTRY = new Set(['command', 'keys']);

/** Reads a keybindings file from its text. Never throws. */
export function readKeys(text: string): ReadKeys {
  if (new TextEncoder().encode(text).length > MAX_FILE_BYTES) return { ok: false, issues: [{ code: 'too-big' }] };
  let raw: unknown;
  try { raw = JSON.parse(text); } catch { return { ok: false, issues: [{ code: 'json' }] }; }
  return readKeysValue(raw);
}

/** Reads a keybindings file already parsed from JSON. Never throws. */
export function readKeysValue(raw: unknown): ReadKeys {
  if (!isObject(raw) || typeof raw.version !== 'number' || !Number.isInteger(raw.version) || raw.version < 1 || !Array.isArray(raw.bindings)) return { ok: false, issues: [{ code: 'shape' }] };
  if (raw.version > KEYS_FORMAT) return { ok: false, issues: [{ code: 'newer' }] };
  const issues: KeyIssue[] = [];
  for (const k of Object.keys(raw)) if (!TOP.has(k)) issues.push({ code: 'field', at: shown(k) });
  if (raw.bindings.length > MAX_ENTRIES) return { ok: false, issues: [...issues, { code: 'too-many' }] };
  const entries: KeyEntry[] = [];
  const unknown: string[] = [];
  const seen = new Set<string>();
  raw.bindings.forEach((e, i) => {
    if (!isObject(e) || typeof e.command !== 'string' || !Array.isArray(e.keys)) { issues.push({ code: 'shape' }); return; }
    for (const k of Object.keys(e)) if (!ENTRY.has(k)) issues.push({ code: 'field', at: `bindings[${i}].${shown(k)}` });
    const command = e.command;
    if (!ID.test(command) || command.length > 64) { issues.push({ code: 'command', command: shown(command) }); return; }
    if (NEVER_COMMAND.test(command)) { issues.push({ code: 'request', command }); return; }
    if (seen.has(command)) { issues.push({ code: 'twice', command }); return; }
    seen.add(command);
    const def = commandById(command);
    if (def?.fixed) { issues.push({ code: 'fixed-command', command }); return; }
    if (e.keys.length > MAX_KEYS_PER_COMMAND) { issues.push({ code: 'too-many-keys', command }); return; }
    const keys: string[] = [];
    for (const k of e.keys as unknown[]) {
      const b = parseBinding(k);
      if (!b) { issues.push({ code: 'key', command, key: shown(k) }); continue; }
      const fixed = b.map(fixedReason).find((r) => r !== undefined);
      if (fixed) { issues.push({ code: 'fixed-key', command, key: bindingText(b), reason: fixed }); continue; }
      const text = bindingText(b);
      if (!keys.includes(text)) keys.push(text);
    }
    if (!def) unknown.push(command);
    entries.push({ command, keys });
  });
  if (issues.length > 0) return { ok: false, issues: issues.length > MAX_ISSUES ? [...issues.slice(0, MAX_ISSUES), { code: 'more', count: issues.length - MAX_ISSUES }] : issues };
  return { ok: true, file: { entries }, unknown };
}

/** The file as JSON data (a theme pack's `keybindings` part, ADR-0113 decision 5). */
export function keysObject(file: KeyFile): { version: number; bindings: { command: string; keys: string[] }[] } {
  return { version: KEYS_FORMAT, bindings: file.entries.map((e) => ({ command: e.command, keys: [...e.keys] })) };
}

/** The file's text: what is read back is the same file. Entries keep their order; keys are written in their one spelling. */
export function writeKeys(file: KeyFile): string {
  return `${JSON.stringify(keysObject(file), null, 2)}\n`;
}

/** The file with one command's keys set (`undefined`: back to its defaults, the entry removed). Pure. */
export function withKeys(file: KeyFile, command: string, keys: readonly string[] | undefined): KeyFile {
  const rest = file.entries.filter((e) => e.command !== command);
  if (keys === undefined) return { entries: rest };
  const at = file.entries.findIndex((e) => e.command === command);
  const entry = { command, keys: [...keys] };
  return { entries: at < 0 ? [...rest, entry] : [...file.entries.slice(0, at), entry, ...file.entries.slice(at + 1)] };
}

export const EMPTY_FILE: KeyFile = { entries: [] };

/**
 * Another file's keys merged over the person's own (a theme pack's keybindings, ADR-0113 decision 5): each command the other file names
 * takes its keys from it, the rest stay. Pure; what it would change is said before anything is kept:
 * - `replaced`: commands the person had set themselves that the other file sets differently (theirs are lost);
 * - `clashes`: keys that would then belong to two commands and did not before;
 * - `issues`: the merged file could not be kept as it is (over the entry or size cap), so it must not be stored: it would be refused on
 *   the next load and the person's own keys with it.
 */
export interface Merge {
  file: KeyFile;
  unknown: string[];
  replaced: { command: string; mine: readonly string[]; theirs: readonly string[] }[];
  clashes: Conflict[];
  issues: KeyIssue[];
}
export function mergeKeys(mine: KeyFile, mineUnknown: readonly string[], theirs: KeyFile, theirsUnknown: readonly string[]): Merge {
  let file = mine;
  const replaced: Merge['replaced'] = [];
  for (const e of theirs.entries) {
    const had = mine.entries.find((m) => m.command === e.command);
    if (had && had.keys.join('\n') !== e.keys.join('\n')) replaced.push({ command: e.command, mine: had.keys, theirs: e.keys });
    file = withKeys(file, e.command, e.keys);
  }
  const key = (c: Conflict) => `${c.kind} ${[c.a, c.b].sort().join(' ')} ${c.keys}`;
  const before = new Set(conflicts(keymap(mine)).map(key));
  const clashes = conflicts(keymap(file)).filter((c) => !before.has(key(c)));
  const unknown = [...new Set([...mineUnknown, ...theirsUnknown])].filter((u) => file.entries.some((e) => e.command === u));
  const again = readKeys(writeKeys(file));
  return { file, unknown, replaced, clashes, issues: again.ok ? [] : again.issues };
}

// ---- The keys in force ----------------------------------------------------------------------------------------------------------------

export interface Bound { command: string; binding: Binding; text: string; /** From the person's file, not a default. */ own: boolean }

/**
 * Every key in force, in the order a key press looks them up: the person's own first (in the file's order: a choice they made wins over
 * a default), then the defaults of the commands the file does not name. A fixed command keeps its defaults whatever the file says, and a
 * command this window does not know binds nothing.
 */
export function keymap(file: KeyFile, defs: readonly CommandDef[] = COMMANDS): Bound[] {
  const out: Bound[] = [];
  const named = new Set<string>();
  for (const e of file.entries) {
    const def = defs.find((d) => d.id === e.command);
    if (!def || def.fixed) continue;
    named.add(e.command);
    for (const text of e.keys) {
      const binding = parseBinding(text);
      if (binding) out.push({ command: e.command, binding, text: bindingText(binding), own: true });
    }
  }
  for (const d of defs) {
    if (named.has(d.id)) continue;
    for (const text of d.keys) {
      const binding = parseBinding(text);
      if (binding) out.push({ command: d.id, binding, text: bindingText(binding), own: false });
    }
  }
  return out;
}

/** A command's keys in force, in order. */
export const keysOf = (map: readonly Bound[], command: string): Binding[] => map.filter((b) => b.command === command).map((b) => b.binding);

/**
 * Two commands that want the same keys: the same key string (`same`), or one that is the start of the other's chord (`prefix`: Ctrl+K
 * and Ctrl+K Ctrl+S, where the chord waits for its second key and the shorter one can no longer act). Shown in Settings > Keyboard.
 */
export interface Conflict { a: string; b: string; keys: string; kind: 'same' | 'prefix' }
export function conflicts(map: readonly Bound[]): Conflict[] {
  const out: Conflict[] = [];
  for (let i = 0; i < map.length; i++) {
    for (let j = i + 1; j < map.length; j++) {
      const x = map[i] as Bound;
      const y = map[j] as Bound;
      if (x.command === y.command) continue;
      if (x.text === y.text) out.push({ a: x.command, b: y.command, keys: x.text, kind: 'same' });
      else if (startsWith(y.binding, x.binding) || startsWith(x.binding, y.binding)) {
        const short = x.binding.length < y.binding.length ? x : y;
        out.push({ a: x.command, b: y.command, keys: short.text, kind: 'prefix' });
      }
    }
  }
  return out;
}
