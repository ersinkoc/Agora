// The text readers theme import needs (docs/ux/themes.md "Import from another app"): a subset of TOML, of YAML, Ghostty's
// `key = value`, JSON with comments, and Apple's property lists. Each reads untrusted text of at most the theme file cap, line by line
// with anchored patterns, never recurses without a bound, never throws, and keeps what it read in a `Map` keyed by a dotted path, so a
// key such as `__proto__` is only a string. They read the shapes colour files use, not the whole languages: what they cannot read is
// skipped and named, and the converters say so in their notes. Nothing here touches the network; the property list goes through the
// browser's own XML parser, which loads no external entity or DTD, and text that declares an entity is refused before it is parsed.

// The most keys a reader keeps, and the longest key: a hostile file cannot grow a reader's map past this.
export const MAX_KEYS = 4096;
const MAX_KEY = 200;

export type Flat = Map<string, string>;
// `copied` (YAML): the keys an alias copied from, so a caller can tell they were read.
export interface Read { values: Flat; skipped: string[]; copied?: Set<string> }

const add = (out: Read, key: string, value: string) => {
  if (key.length > MAX_KEY || out.values.size >= MAX_KEYS) return;
  out.values.set(key, value);
};
const skip = (out: Read, key: string) => { if (out.skipped.length < MAX_KEYS) out.skipped.push(key.slice(0, MAX_KEY)); };
export const lines = (text: string) => text.replace(/^﻿/, '').split(/\r\n|\r|\n/);

// The end of a quoted string that starts at `from` (the index of its quote), or -1. A backslash escapes the next character in a
// double-quoted string; a single-quoted one has no escapes (TOML literal strings, YAML single quotes).
function closing(s: string, from: number): number {
  const q = s[from];
  for (let i = from + 1; i < s.length; i++) {
    if (q === '"' && s[i] === '\\') { i++; continue; }
    if (s[i] === q) return i;
  }
  return -1;
}
// A quoted string's text: the common escapes of a double-quoted string, nothing in a single-quoted one.
function unquoteAt(s: string, from: number, end: number): string {
  const body = s.slice(from + 1, end);
  if (s[from] !== '"') return body;
  return body.replace(/\\(["\\/bfnrt])/g, (_, c: string) => ({ b: '\b', f: '\f', n: '\n', r: '\r', t: '\t' } as Record<string, string>)[c] ?? c);
}
// A line without its comment (`#` outside quotes; YAML needs a space or the line start before it).
function uncomment(s: string, yaml = false): string {
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '"' || c === "'") { const e = closing(s, i); if (e < 0) return s; i = e; continue; }
    if (c === '#' && (!yaml || i === 0 || s[i - 1] === ' ' || s[i - 1] === '\t')) return s.slice(0, i);
  }
  return s;
}
// The index of `ch` in `s` outside quotes, or -1.
function indexOutside(s: string, ch: string, from = 0): number {
  for (let i = from; i < s.length; i++) {
    const c = s[i];
    if (c === '"' || c === "'") { const e = closing(s, i); if (e < 0) return -1; i = e; continue; }
    if (c === ch) return i;
  }
  return -1;
}
// A scalar as text: a quoted string's text, or the bare word as written.
function scalar(v: string): string {
  const s = v.trim();
  if ((s.startsWith('"') || s.startsWith("'")) && closing(s, 0) === s.length - 1 && s.length >= 2) return unquoteAt(s, 0, s.length - 1);
  return s;
}

// ---- TOML (Alacritty) ---------------------------------------------------------------------------------------------------------

// A dotted key's parts (`colors.primary`, `"a.b".c`), or undefined when it is not one.
function keyParts(k: string): string[] | undefined {
  const parts: string[] = [];
  let i = 0;
  const s = k.trim();
  if (s.length > MAX_KEY) return undefined;
  while (i < s.length) {
    while (s[i] === ' ' || s[i] === '\t') i++;
    if (s[i] === '"' || s[i] === "'") {
      const e = closing(s, i);
      if (e < 0) return undefined;
      parts.push(unquoteAt(s, i, e));
      i = e + 1;
    } else {
      const m = /^[A-Za-z0-9_-]+/.exec(s.slice(i));
      if (!m) return undefined;
      parts.push(m[0]);
      i += m[0].length;
    }
    while (s[i] === ' ' || s[i] === '\t') i++;
    if (i < s.length) { if (s[i] !== '.') return undefined; i++; }
  }
  return parts.length > 0 ? parts : undefined;
}

// The pieces of an inline table's body (`a = "x", b = 'y'`), split on commas outside quotes and braces.
function inlineEntries(body: string): string[] | undefined {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < body.length; i++) {
    const c = body[i];
    if (c === '"' || c === "'") { const e = closing(body, i); if (e < 0) return undefined; i = e; continue; }
    if (c === '{' || c === '[') depth++;
    else if (c === '}' || c === ']') depth--;
    else if (c === ',' && depth === 0) { out.push(body.slice(start, i)); start = i + 1; }
  }
  out.push(body.slice(start));
  return out.map((x) => x.trim()).filter((x) => x.length > 0);
}

// How deep the brackets of `s` go by its end (strings skipped): what a multi-line array or inline table still has open.
function openBrackets(s: string): number {
  let depth = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '"' || c === "'") { const e = closing(s, i); if (e < 0) return depth; i = e; continue; }
    if (c === '[' || c === '{') depth++;
    else if (c === ']' || c === '}') depth--;
  }
  return depth;
}

// TOML as Alacritty writes it: `[table]` headers, `key = value` with dotted or quoted keys, strings, numbers and booleans, and
// inline tables one level deep. Arrays, arrays of tables and multi-line strings are skipped (named in `skipped`).
export function readToml(text: string): Read {
  const out: Read = { values: new Map(), skipped: [] };
  let table: string[] | undefined = [];
  let open = 0; // brackets still open from a skipped multi-line value
  let fence: string | undefined; // a skipped multi-line string's closing quotes
  for (const raw of lines(text)) {
    if (fence) { if (raw.includes(fence)) fence = undefined; continue; }
    const line = uncomment(raw).trim();
    if (open > 0) { open += openBrackets(line); continue; }
    if (line === '') continue;
    if (line.startsWith('[[')) { table = undefined; skip(out, line.slice(2, MAX_KEY).replace(/\]+$/, '').trim()); continue; }
    if (line.startsWith('[')) {
      const m = /^\[([^\[\]]*)\]$/.exec(line);
      table = m ? keyParts(m[1] as string) : undefined;
      if (!table) skip(out, line.slice(0, MAX_KEY));
      continue;
    }
    const eq = indexOutside(line, '=');
    if (eq <= 0) { skip(out, line.slice(0, 40)); continue; }
    const parts = keyParts(line.slice(0, eq));
    const value = line.slice(eq + 1).trim();
    if (!parts || !table) { skip(out, line.slice(0, eq).trim()); continue; }
    const key = [...table, ...parts].join('.');
    if (value.startsWith('"""') || value.startsWith("'''")) {
      const q = value.slice(0, 3);
      if (value.indexOf(q, 3) < 0) fence = q;
      skip(out, key);
    } else if (value.startsWith('[')) {
      open = openBrackets(value);
      skip(out, key);
    } else if (value.startsWith('{')) {
      open = openBrackets(value);
      if (open !== 0 || !value.endsWith('}')) { skip(out, key); continue; }
      const entries = inlineEntries(value.slice(1, -1));
      if (!entries) { skip(out, key); continue; }
      for (const e of entries) {
        const i = indexOutside(e, '=');
        const sub = i > 0 ? keyParts(e.slice(0, i)) : undefined;
        const v = i > 0 ? e.slice(i + 1).trim() : '';
        if (!sub || v.startsWith('{') || v.startsWith('[')) skip(out, `${key}.${sub?.join('.') ?? '?'}`);
        else add(out, `${key}.${sub.join('.')}`, scalar(v));
      }
    } else add(out, key, scalar(value));
  }
  return out;
}

// ---- YAML (Alacritty before 0.13) ----------------------------------------------------------------------------------------------

// The YAML that Alacritty's old colour files are: nested maps by indentation, scalars, comments, and anchors and aliases on maps
// (`colors: *dracula`, `<<: *base`). Lists, block scalars and flow collections are skipped. An alias copies at most what is left of
// `MAX_KEYS`, so aliases of aliases cannot grow without bound.
const MAX_ALIASES = 64;
export function readYaml(text: string): Read {
  const copied = new Set<string>();
  const out: Read = { values: new Map(), skipped: [], copied };
  const stack: { indent: number; path: string }[] = [];
  const anchors = new Map<string, string>();
  let aliases = 0; // each alias walks what was read so far: at most MAX_ALIASES of them are followed
  let skipBelow = -1; // the indent of a skipped list or block scalar: deeper lines belong to it
  const alias = (name: string, to: string) => {
    const from = anchors.get(name);
    if (from === undefined || ++aliases > MAX_ALIASES) { skip(out, to); return; }
    const prefix = from === '' ? '' : `${from}.`;
    for (const [k, v] of [...out.values]) {
      if (out.values.size >= MAX_KEYS) break;
      if (!k.startsWith(prefix)) continue;
      add(out, to === '' ? k.slice(prefix.length) : `${to}.${k.slice(prefix.length)}`, v);
      copied.add(k);
    }
  };
  for (const raw of lines(text)) {
    const body = raw.replace(/\t/g, ' ');
    const indent = body.length - body.trimStart().length;
    const line = uncomment(body, true).trim();
    if (line === '' || line === '---' || line === '...') continue;
    if (skipBelow >= 0) { if (indent > skipBelow) continue; skipBelow = -1; }
    while (stack.length > 0 && (stack[stack.length - 1] as { indent: number }).indent >= indent) stack.pop();
    const parent = stack.length > 0 ? (stack[stack.length - 1] as { path: string }).path : '';
    if (line.startsWith('- ') || line === '-' || line.startsWith('[') || line.startsWith('{')) { skipBelow = indent; skip(out, parent || line.slice(0, 40)); continue; }
    let key: string;
    let rest: string;
    if (line.startsWith('"') || line.startsWith("'")) {
      const e = closing(line, 0);
      if (e < 0 || line[e + 1] !== ':') { skip(out, line.slice(0, 40)); continue; }
      key = unquoteAt(line, 0, e);
      rest = line.slice(e + 2);
    } else {
      const m = /^([^:]{1,200}?):(?:\s|$)/.exec(line);
      if (!m) { skip(out, line.slice(0, 40)); continue; }
      key = (m[1] as string).trim();
      rest = line.slice(m[0].length);
    }
    const value = rest.trim();
    const path = key === '<<' ? parent : parent ? `${parent}.${key}` : key;
    if (value === '') { stack.push({ indent, path }); continue; }
    if (value.startsWith('&')) {
      const m = /^&([A-Za-z0-9_-]{1,100})(?:\s+(.*))?$/.exec(value);
      if (!m) { skip(out, path); continue; }
      anchors.set(m[1] as string, path);
      if (m[2] === undefined || m[2] === '') stack.push({ indent, path });
      else add(out, path, scalar(m[2]));
      continue;
    }
    if (value.startsWith('*')) { alias(value.slice(1).trim(), path); continue; }
    if (value.startsWith('|') || value.startsWith('>') || value.startsWith('[') || value.startsWith('{')) { skipBelow = indent; skip(out, path); continue; }
    if (key !== '<<') add(out, path, scalar(value));
  }
  return out;
}

// ---- Ghostty --------------------------------------------------------------------------------------------------------------------

// Ghostty's config and theme files: `key = value` per line, `#` comments on their own line, a key that may repeat (`palette`).
// Every pair is kept in order, so `palette = 0=#1d1f21` and the next `palette` are both read.
export function readKeyValue(text: string): { pairs: [string, string][]; skipped: string[] } {
  const pairs: [string, string][] = [];
  const skipped: string[] = [];
  for (const raw of lines(text)) {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) continue;
    const m = /^([A-Za-z0-9_-]{1,100})\s*=\s*(.*)$/.exec(line);
    if (!m) { if (skipped.length < MAX_KEYS) skipped.push(line.slice(0, 40)); continue; }
    if (pairs.length < MAX_KEYS) pairs.push([m[1] as string, scalar(m[2] as string)]);
  }
  return { pairs, skipped };
}

// ---- JSON with comments (VS Code, Windows Terminal) ----------------------------------------------------------------------------

// JSON as VS Code and Windows Terminal write their files: `//` and `/* */` comments and trailing commas allowed. One pass removes
// them outside strings, then the browser's JSON parser reads the rest. Undefined when it is not JSON even so.
export function readJsonc(text: string): unknown {
  const s = text.replace(/^﻿/, '');
  let out = '';
  let i = 0;
  let chunk = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === '"') {
      const e = closing(s, i);
      if (e < 0) return undefined;
      i = e + 1;
      continue;
    }
    if (c === '/' && s[i + 1] === '/') {
      out += s.slice(chunk, i);
      const nl = s.indexOf('\n', i);
      i = nl < 0 ? s.length : nl;
      chunk = i;
      continue;
    }
    if (c === '/' && s[i + 1] === '*') {
      out += s.slice(chunk, i);
      const end = s.indexOf('*/', i + 2);
      i = end < 0 ? s.length : end + 2;
      chunk = i;
      continue;
    }
    if (c === ',') {
      // A trailing comma: the next character that is not space or a comment closes the object or array.
      let j = i + 1;
      for (;;) {
        while (j < s.length && /\s/.test(s[j] as string)) j++;
        if (s[j] === '/' && s[j + 1] === '/') { const nl = s.indexOf('\n', j); j = nl < 0 ? s.length : nl; continue; }
        if (s[j] === '/' && s[j + 1] === '*') { const end = s.indexOf('*/', j + 2); j = end < 0 ? s.length : end + 2; continue; }
        break;
      }
      if (s[j] === '}' || s[j] === ']') { out += s.slice(chunk, i); i++; chunk = i; continue; }
    }
    i++;
  }
  out += s.slice(chunk);
  try { return JSON.parse(out); } catch { return undefined; }
}

// ---- Property lists (iTerm2) ---------------------------------------------------------------------------------------------------

const MAX_DEPTH = 8;
const MAX_ELEMENTS = 20_000;
export type PlistValue = { kind: 'dict'; entries: Map<string, PlistValue> } | { kind: 'number'; value: number } | { kind: 'string'; value: string } | { kind: 'other' };

// An XML property list's top dictionary, two levels deep (iTerm2's colours are a dictionary of dictionaries), or a reason it is not
// one. Text that declares an entity or a DTD of its own is refused before the parser sees it (no expansion, the same answer in every
// browser); an external DTD (`<!DOCTYPE plist PUBLIC … "http://…">`, which every iTerm2 file has) is never loaded by the browser.
export function readPlist(text: string): { ok: true; top: Map<string, PlistValue> } | { ok: false; why: 'entities' | 'unreadable' } {
  if (/<!ENTITY/i.test(text)) return { ok: false, why: 'entities' };
  const lower = text.toLowerCase();
  const doctype = lower.indexOf('<!doctype');
  if (doctype >= 0) {
    const end = lower.indexOf('>', doctype);
    if (lower.slice(doctype, end < 0 ? lower.length : end).includes('[')) return { ok: false, why: 'entities' };
  }
  // A colour file is two dictionaries deep and a few hundred elements long: deeper nesting or a flood of elements is not one, and is
  // refused before the parser builds a document of it.
  let depth = 0;
  let elements = 0;
  for (const m of text.matchAll(/<(\/?)(dict|array)\b|<[A-Za-z]/g)) {
    if (++elements > MAX_ELEMENTS) return { ok: false, why: 'unreadable' };
    if (m[2] === undefined) continue;
    depth += m[1] === '/' ? -1 : 1;
    if (depth > MAX_DEPTH) return { ok: false, why: 'unreadable' };
  }
  const Parser = (globalThis as { DOMParser?: typeof DOMParser }).DOMParser;
  if (!Parser) return { ok: false, why: 'unreadable' };
  let doc: Document;
  try { doc = new Parser().parseFromString(text.replace(/^﻿/, ''), 'application/xml'); } catch { return { ok: false, why: 'unreadable' }; }
  // A parse error is a <parsererror> element in the document, not an exception.
  if (doc.getElementsByTagName('parsererror').length > 0) return { ok: false, why: 'unreadable' };
  const root = doc.documentElement;
  const dict = root?.nodeName === 'plist' ? [...root.children].find((c) => c.nodeName === 'dict') : root?.nodeName === 'dict' ? root : undefined;
  if (!dict) return { ok: false, why: 'unreadable' };
  return { ok: true, top: dictOf(dict, 1) };
}
function dictOf(el: Element, depth: number): Map<string, PlistValue> {
  const out = new Map<string, PlistValue>();
  const kids = [...el.children];
  for (let i = 0; i + 1 < kids.length && out.size < MAX_KEYS; i++) {
    const k = kids[i] as Element;
    if (k.nodeName !== 'key') continue;
    const v = kids[i + 1] as Element;
    i++;
    const key = (k.textContent ?? '').slice(0, MAX_KEY);
    if (v.nodeName === 'dict') out.set(key, depth > 0 ? { kind: 'dict', entries: dictOf(v, depth - 1) } : { kind: 'other' });
    else if (v.nodeName === 'real' || v.nodeName === 'integer') {
      const n = Number((v.textContent ?? '').trim());
      out.set(key, Number.isFinite(n) ? { kind: 'number', value: n } : { kind: 'other' });
    } else if (v.nodeName === 'string') out.set(key, { kind: 'string', value: (v.textContent ?? '').slice(0, MAX_KEY) });
    else out.set(key, { kind: 'other' });
  }
  return out;
}
