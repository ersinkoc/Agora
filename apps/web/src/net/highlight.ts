// Helpers around syntax highlighting that do not need the highlighter itself (it is loaded only when a file is shown): which
// language a file is in, the highlighted HTML of a whole file as one string per line, and a mark laid over a range of a line's
// text. The HTML these take is the highlighter's: text escaped as &amp; &lt; &gt; &quot; &#x27;, and <span class="..."> tags only.

/** Languages loaded by `highlight/hl.ts`, by file name: a known name first (`Dockerfile`), then the extension. */
const BY_NAME: Record<string, string> = { dockerfile: 'dockerfile', makefile: 'makefile', 'cargo.lock': 'ini', '.gitignore': 'bash' };
const BY_EXT: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
  json: 'json', jsonc: 'json',
  rs: 'rust', py: 'python', go: 'go', java: 'java', kt: 'kotlin', kts: 'kotlin', swift: 'swift',
  c: 'c', h: 'c', cc: 'cpp', cpp: 'cpp', cxx: 'cpp', hpp: 'cpp', cs: 'csharp',
  rb: 'ruby', php: 'php',
  sh: 'bash', bash: 'bash', zsh: 'bash', ps1: 'powershell', psm1: 'powershell',
  yml: 'yaml', yaml: 'yaml', toml: 'ini', ini: 'ini', cfg: 'ini', ftl: 'ini',
  md: 'markdown', markdown: 'markdown',
  css: 'css', scss: 'scss',
  html: 'xml', htm: 'xml', xml: 'xml', svg: 'xml', vue: 'xml',
  sql: 'sql', proto: 'protobuf', diff: 'diff', patch: 'diff',
};

export function languageOf(path: string): string | undefined {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase();
  if (BY_NAME[name]) return BY_NAME[name];
  const dot = name.lastIndexOf('.');
  return dot > 0 ? BY_EXT[name.slice(dot + 1)] : undefined;
}

const TAG = /<span class="[^"<>]*">|<\/span>/g;

/** A whole file's highlighted HTML as one string per line, each with its spans closed at the end and reopened at the start of the
 *  next line (a comment or a string that runs over several lines stays coloured). */
export function splitHighlighted(html: string): string[] {
  const out: string[] = [];
  const open: string[] = [];
  let line = '';
  let at = 0;
  const text = (s: string) => {
    const parts = s.split('\n');
    parts.forEach((part, i) => {
      if (i > 0) {
        out.push(line + '</span>'.repeat(open.length));
        line = open.join('');
      }
      line += part.endsWith('\r') && i < parts.length - 1 ? part.slice(0, -1) : part;
    });
  };
  for (const m of html.matchAll(TAG)) {
    text(html.slice(at, m.index));
    if (m[0] === '</span>') open.pop(); else open.push(m[0]);
    line += m[0];
    at = (m.index ?? 0) + m[0].length;
  }
  text(html.slice(at));
  out.push(line + '</span>'.repeat(open.length));
  // A file ending in a line break has no line after it.
  if (out.length > 1 && out[out.length - 1] === open.join('') + '</span>'.repeat(open.length) && html.endsWith('\n')) out.pop();
  return out;
}

const ENTITY = /^&(?:amp|lt|gt|quot|#x27|#39|#x2F);/;

/** `html` with each of `ranges` (offsets in the line's text, not counting tags; [start, end), sorted, not overlapping) wrapped in
 *  `<mark class="...">`. A range that crosses a span boundary is split there, so the tags stay nested. */
export function markRanges(html: string, ranges: { start: number; end: number; cls: string }[]): string {
  if (ranges.length === 0) return html;
  let out = '';
  let pos = 0; // offset in the text
  let r = 0;
  let marking = false;
  const openMark = () => { out += `<mark class="${ranges[r]?.cls ?? ''}">`; marking = true; };
  const closeMark = () => { out += '</mark>'; marking = false; };
  let i = 0;
  while (i < html.length) {
    if (html[i] === '<') {
      const end = html.indexOf('>', i);
      if (marking) closeMark();
      out += html.slice(i, end + 1);
      i = end + 1;
      const cur = ranges[r];
      if (cur && pos >= cur.start && pos < cur.end) openMark();
      continue;
    }
    const cur = ranges[r];
    if (cur && pos === cur.start && !marking && cur.end > cur.start) openMark();
    const ent = html[i] === '&' ? ENTITY.exec(html.slice(i, i + 7)) : null;
    const ch = ent ? ent[0] : (html[i] as string);
    out += ch;
    i += ch.length;
    pos++;
    if (cur && pos === cur.end) {
      if (marking) closeMark();
      r++;
      const next = ranges[r];
      if (next && pos === next.start && next.end > next.start) openMark();
    }
  }
  if (marking) closeMark();
  return out;
}
