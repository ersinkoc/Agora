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

const LINK = /(\[[^\]\n]+\]\([^)\s]+\)|https?:\/\/[^\s<>()[\]"']+)/;

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
    for (const piece of part.split(LINK)) {
      const md = /^\[([^\]\n]+)\]\(([^)\s]+)\)$/.exec(piece);
      if (md) {
        const href = safeLink(md[2] as string);
        if (href) push('link', md[1] as string, href); else push('text', piece);
      } else if (/^https?:\/\//.test(piece)) {
        // A sentence's closing punctuation is not part of the address.
        const trimmed = piece.replace(/[.,;:!?]+$/, '');
        const href = safeLink(trimmed);
        if (href) { push('link', trimmed, href); push('text', piece.slice(trimmed.length)); } else push('text', piece);
      } else bolds(piece);
    }
  }
  return out;
}

const FENCE = /^\s*(`{3,}|~{3,})\s*([\w+.-]*)\s*$/;
const CLOSE = /^\s*(`{3,}|~{3,})\s*$/;
const HEADING = /^ {0,3}#{1,6}([ \t].*)$/;
const ITEM = /^(\s*)([-*+]|(\d+)[.)])\s+(.*)$/;
const QUOTE = /^\s*>\s?(.*)$/;
const TABLE = /^\s*\|.*\|\s*$/;
const SEPARATOR = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

const cells = (row: string): string[] => row.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map((c) => c.trim());

// A table with a header and a separator row becomes cells; anything else stays as the text it was.
function table(rows: string[]): Block {
  const text = rows.join('\n');
  if (rows.length >= 2 && SEPARATOR.test(rows[1] as string)) {
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
    const fence = FENCE.exec(line);
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
    const heading = HEADING.exec(line);
    if (heading) {
      endPara();
      const title = (heading[1] ?? '').replace(/[ \t]+#+[ \t]*$/, '').trim();
      out.push({ kind: 'heading', inlines: inlines(title) });
      continue;
    }
    if (TABLE.test(line)) {
      endPara();
      const rows = [line];
      while (i + 1 < lines.length && TABLE.test(lines[i + 1] as string)) rows.push(lines[++i] as string);
      out.push(table(rows));
      continue;
    }
    const item = ITEM.exec(line);
    if (item && !para.length) {
      const ordered = item[3] !== undefined;
      const items: string[] = [item[4] ?? ''];
      // Items of the same list follow; an indented line that is not an item continues the one before it.
      while (i + 1 < lines.length) {
        const next = lines[i + 1] as string;
        const m = ITEM.exec(next);
        if (m && (m[3] !== undefined) === ordered) { items.push(m[4] ?? ''); i++; }
        else if (m && (m[1] ?? '').length > 0) { items.push(m[4] ?? ''); i++; }
        else if (next.trim() && /^\s{2,}/.test(next)) { items[items.length - 1] += `\n${next.trim()}`; i++; }
        else break;
      }
      out.push({ kind: 'list', ordered, start: ordered ? Number(item[3]) || 1 : 1, items: items.map(inlines) });
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
