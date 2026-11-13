// The little Markdown agents write, read into blocks the window draws itself: paragraphs, headings, lists, quotes, code blocks and
// tables, with inline code and bold. No HTML is ever produced or passed through: what is not understood stays as text. Pure.

export interface Inline { kind: 'text' | 'code' | 'strong'; text: string }
export type Block =
  | { kind: 'p'; inlines: Inline[] }
  | { kind: 'heading'; inlines: Inline[] }
  | { kind: 'list'; ordered: boolean; start: number; items: Inline[][] }
  | { kind: 'quote'; inlines: Inline[] }
  | { kind: 'code'; lang: string; text: string }
  | { kind: 'table'; text: string };

// `code` first (nothing inside it is read), then **bold** in what is left.
export function inlines(text: string): Inline[] {
  const out: Inline[] = [];
  const push = (kind: Inline['kind'], s: string) => {
    if (!s) return;
    const last = out[out.length - 1];
    if (last && last.kind === kind && kind === 'text') last.text += s;
    else out.push({ kind, text: s });
  };
  const parts = text.split(/(`[^`\n]+`)/);
  for (const part of parts) {
    if (part.length > 2 && part.startsWith('`') && part.endsWith('`')) { push('code', part.slice(1, -1)); continue; }
    const bold = part.split(/(\*\*[^*\n]+\*\*)/);
    for (const b of bold) {
      if (b.length > 4 && b.startsWith('**') && b.endsWith('**')) push('strong', b.slice(2, -2));
      else push('text', b);
    }
  }
  return out;
}

const FENCE = /^\s*(```|~~~)\s*([\w+.-]*)\s*$/;
const HEADING = /^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/;
const ITEM = /^(\s*)([-*+]|(\d+)[.)])\s+(.*)$/;
const QUOTE = /^\s*>\s?(.*)$/;
const TABLE = /^\s*\|.*\|\s*$/;

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
      for (i++; i < lines.length && !(lines[i] as string).trim().startsWith(fence[1] as string); i++) body.push(lines[i] as string);
      out.push({ kind: 'code', lang: fence[2] ?? '', text: body.join('\n') });
      continue;
    }
    if (!line.trim()) { endPara(); continue; }
    const heading = HEADING.exec(line);
    if (heading) { endPara(); out.push({ kind: 'heading', inlines: inlines(heading[1] ?? '') }); continue; }
    if (TABLE.test(line)) {
      endPara();
      const rows = [line];
      while (i + 1 < lines.length && TABLE.test(lines[i + 1] as string)) rows.push(lines[++i] as string);
      out.push({ kind: 'table', text: rows.join('\n') });
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
