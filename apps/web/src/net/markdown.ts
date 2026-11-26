// The little Markdown agents write, read into blocks the window draws itself: paragraphs, headings, lists, quotes, code blocks and
// tables, with inline code, bold and links. No HTML is ever produced or passed through: what is not understood stays as text. Pure.
import { safeLink } from '../terminal/links';

/** `href` is only for a link: an address that passed `safeLink` (http or https, no credentials). */
export interface Inline { kind: 'text' | 'code' | 'strong' | 'link'; text: string; href?: string }
export type Block =
  | { kind: 'p'; inlines: Inline[] }
  | { kind: 'heading'; inlines: Inline[] }
  | { kind: 'list'; ordered: boolean; start: number; items: Inline[][] }
  | { kind: 'quote'; inlines: Inline[] }
  | { kind: 'code'; lang: string; text: string }
  /** `text` is the table as written (the fallback); `head` and `rows` are its cells when it has a header and a separator row. */
  | { kind: 'table'; text: string; head?: Inline[][]; rows?: Inline[][][] };

const BARE_LINK = /https?:\/\/[^\s<>()[\]"']+/y;
const SPACE = /\s/;

// What `text.split(/(\[[^\]\n]+\]\([^)\s]+\)|https?:\/\/[^\s<>()[\]"']+)/)` returns (text, link, text, ..., text), found in one pass: the
// pattern took time with the square of a run of `[` (F2082). The next `]` (or line break) and the next `)` (or white space) are each
// looked for once, not again from every `[`.
function splitLinks(s: string): string[] {
  const out: string[] = [];
  let from = 0;
  let close = -1; // the first `]` or line break after the last `[` looked at
  let shut = -1; // the first `)` or white space at or after the last address start looked at
  for (let i = 0; i < s.length; i++) {
    let end = -1;
    if (s[i] === '[') {
      if (close <= i) { close = i + 1; while (close < s.length && s[close] !== ']' && s[close] !== '\n') close++; }
      if (close > i + 1 && s[close] === ']' && s[close + 1] === '(') {
        const at = close + 2;
        if (shut < at) { shut = at; while (shut < s.length && s[shut] !== ')' && !SPACE.test(s[shut] as string)) shut++; }
        if (shut > at && s[shut] === ')') end = shut + 1;
      }
    } else if (s[i] === 'h') {
      BARE_LINK.lastIndex = i;
      if (BARE_LINK.test(s)) end = BARE_LINK.lastIndex;
    }
    if (end > 0) { out.push(s.slice(from, i), s.slice(i, end)); from = end; i = end - 1; }
  }
  out.push(s.slice(from));
  return out;
}

// `code` first (nothing inside it is read), then links, then **bold** in what is left. A link is drawn only when its address is a plain
// web address: [x](javascript:...) stays as the text it was written as, never as a link.
export function inlines(text: string): Inline[] {
  const out: Inline[] = [];
  const push = (kind: Inline['kind'], s: string, href?: string) => {
    if (!s) return;
    const last = out[out.length - 1];
    if (last && last.kind === kind && kind === 'text') last.text += s;
    else out.push(href ? { kind, text: s, href } : { kind, text: s });
  };
  const bolds = (part: string) => {
    for (const b of part.split(/(\*\*[^*\n]+\*\*)/)) {
      if (b.length > 4 && b.startsWith('**') && b.endsWith('**')) push('strong', b.slice(2, -2));
      else push('text', b);
    }
  };
  for (const part of text.split(/(`[^`\n]+`)/)) {
    if (part.length > 2 && part.startsWith('`') && part.endsWith('`')) { push('code', part.slice(1, -1)); continue; }
    for (const piece of splitLinks(part)) {
      const md = /^\[([^\]\n]+)\]\(([^)\s]+)\)$/.exec(piece);
      if (md) {
        const href = safeLink(md[2] as string);
        if (href) push('link', md[1] as string, href); else push('text', piece);
      } else if (/^https?:\/\//.test(piece)) {
        // A sentence's closing punctuation is not part of the address (cut by hand: `/[.,;:!?]+$/` took time with the square of a run
        // of dots, F2082).
        let cut = piece.length;
        while (cut > 0 && '.,;:!?'.includes(piece[cut - 1] as string)) cut--;
        const trimmed = piece.slice(0, cut);
        const href = safeLink(trimmed);
        if (href) { push('link', trimmed, href); push('text', piece.slice(trimmed.length)); } else push('text', piece);
      } else bolds(piece);
    }
  }
  return out;
}

// Read on the trimmed line (F2080): two runs of `\s*` that meet backtrack with the square of a run of spaces in what an agent writes.
const FENCE = /^(`{3,}|~{3,})\s*([\w+.-]*)$/;
const CLOSE = /^\s*(`{3,}|~{3,})\s*$/;
const HEADING_START = /^ {0,3}#{1,6}[ \t]+/;
const ITEM_START = /^(\s*)([-*+]|(\d+)[.)])\s+/;
const QUOTE = /^\s*>\s?(.*)$/;
const TABLE = /^\s*\|.*\|\s*$/;
const SEPARATOR_CELL = /^:?-+:?$/;

// A heading's text: what follows the `#`s, without the closing `#`s and the spaces around them; undefined for a line that is not one. Cut
// by hand, not by one pattern, which backtracked with the cube of a run of spaces (F2080). Its text holds no line break, as `.` does not.
function headingText(line: string): string | undefined {
  const start = HEADING_START.exec(line)?.[0].length;
  if (start === undefined) return undefined;
  const blank = (c: string | undefined) => c === ' ' || c === '\t';
  let end = line.length;
  while (end > start && blank(line[end - 1])) end--;
  // A closing run of `#` goes only when a space or tab comes before it (`# C#` keeps its `#`).
  let hashes = end;
  while (hashes > start && line[hashes - 1] === '#') hashes--;
  if (hashes < end && (hashes === start || blank(line[hashes - 1]))) {
    end = hashes;
    while (end > start && blank(line[end - 1])) end--;
  }
  const text = line.slice(start, end);
  return /[\r\n\u2028\u2029]/.test(text) ? undefined : text;
}

// A list item: its indent, its number (an ordered one) and its text; undefined for a line that is not one. Its text holds no line break,
// as `.` does not; cut by hand, as one pattern for the whole line took time with the square of it (F2082).
function listItem(line: string): { indent: string; number?: string; text: string } | undefined {
  const m = ITEM_START.exec(line);
  if (!m) return undefined;
  const text = line.slice(m[0].length);
  return /[\r\n\u2028\u2029]/.test(text) ? undefined : { indent: m[1] ?? '', number: m[3], text };
}

// A table's separator row (`|---|:-:|`): every cell a run of dashes with an optional colon at either end. Cells split by hand (F2080).
function separator(row: string): boolean {
  let t = row.trim();
  if (t.startsWith('|')) t = t.slice(1);
  if (t.endsWith('|')) t = t.slice(0, -1);
  return t.split('|').every((c) => SEPARATOR_CELL.test(c.trim()));
}

const cells = (row: string): string[] => row.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map((c) => c.trim());

// A table with a header and a separator row becomes cells; anything else stays as the text it was.
function table(rows: string[]): Block {
  const text = rows.join('\n');
  if (rows.length >= 2 && separator(rows[1] as string)) {
    return { kind: 'table', text, head: cells(rows[0] as string).map(inlines), rows: rows.slice(2).map((r) => cells(r).map(inlines)) };
  }
  return { kind: 'table', text };
}

export function blocks(text: string): Block[] {
  const out: Block[] = [];
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  let para: string[] = [];
  const endPara = () => {
    if (para.length) out.push({ kind: 'p', inlines: inlines(para.join('\n')) });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] as string;
    const fence = FENCE.exec(line.trim());
    if (fence) {
      endPara();
      const body: string[] = [];
      // A block still being written has no closing fence yet: everything after the opening one is code.
      // It closes on a line that is only a fence of the same character, at least as long as the opening one.
      const open = fence[1] as string;
      const closes = (l: string) => { const m = CLOSE.exec(l)?.[1]; return !!m && m[0] === open[0] && m.length >= open.length; };
      for (i++; i < lines.length && !closes(lines[i] as string); i++) body.push(lines[i] as string);
      out.push({ kind: 'code', lang: fence[2] ?? '', text: body.join('\n') });
      continue;
    }
    if (!line.trim()) { endPara(); continue; }
    const heading = headingText(line);
    if (heading !== undefined) { endPara(); out.push({ kind: 'heading', inlines: inlines(heading) }); continue; }
    if (TABLE.test(line)) {
      endPara();
      const rows = [line];
      while (i + 1 < lines.length && TABLE.test(lines[i + 1] as string)) rows.push(lines[++i] as string);
      out.push(table(rows));
      continue;
    }
    const item = listItem(line);
    if (item && !para.length) {
      const ordered = item.number !== undefined;
      const items: string[] = [item.text];
      // Items of the same list follow; an indented line that is not an item continues the one before it.
      while (i + 1 < lines.length) {
        const next = lines[i + 1] as string;
        const m = listItem(next);
        if (m && (m.number !== undefined) === ordered) { items.push(m.text); i++; }
        else if (m && m.indent.length > 0) { items.push(m.text); i++; }
        else if (next.trim() && /^\s{2,}/.test(next)) { items[items.length - 1] += `\n${next.trim()}`; i++; }
        else break;
      }
      out.push({ kind: 'list', ordered, start: ordered ? Number(item.number) || 1 : 1, items: items.map(inlines) });
      continue;
    }
    const quote = QUOTE.exec(line);
    if (quote && !para.length) {
      const q = [quote[1] ?? ''];
      while (i + 1 < lines.length && QUOTE.test(lines[i + 1] as string)) q.push(QUOTE.exec(lines[++i] as string)?.[1] ?? '');
      out.push({ kind: 'quote', inlines: inlines(q.join('\n')) });
      continue;
    }
    para.push(line);
  }
  endPara();
  return out;
}
