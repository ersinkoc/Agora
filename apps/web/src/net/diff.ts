// Reads the unified diff a node returns (`git.diff`) into files, hunks and numbered lines the review screen can show, and derives
// what that screen needs from them: rows side by side, files by folder, the unchanged lines between hunks, the changed part of a
// changed line. Pure: text in, data out.

export type DiffStatus = 'added' | 'deleted' | 'modified' | 'renamed';

export interface DiffLine {
  kind: 'add' | 'del' | 'ctx' | 'hunk';
  text: string;
  /** Line number in the old file (deleted and unchanged lines). */
  oldNo?: number;
  /** Line number in the new file (added and unchanged lines). */
  newNo?: number;
  /** git's "\ No newline at end of file" followed this line. */
  noEol?: boolean;
}

export interface DiffHunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  /** The text git shows after the second `@@` (often the enclosing function). */
  section: string;
  /** Index of this hunk's `hunk` line in the file's `lines`. */
  at: number;
}

export interface DiffFile {
  path: string;
  /** The name before a rename. */
  oldPath?: string;
  status: DiffStatus;
  binary: boolean;
  added: number;
  removed: number;
  lines: DiffLine[];
  hunks: DiffHunk[];
  /** The diff was cut off inside this file (the node answers at most `MAX_DIFF_BYTES`). */
  partial?: boolean;
}

/** The most a node answers for one diff (`MAX_DIFF_BYTES` in agora-node's control.rs); it cuts at a character boundary. */
export const MAX_DIFF_BYTES = 512 * 1024;

/** Was this diff cut off by the node? It cuts to at most `MAX_DIFF_BYTES`, never more than 3 bytes short of it. */
export function isTruncated(text: string): boolean {
  return text.length * 3 >= MAX_DIFF_BYTES - 3 && new TextEncoder().encode(text).length >= MAX_DIFF_BYTES - 3;
}

const ESCAPES: Record<string, number> = { a: 7, b: 8, t: 9, n: 10, v: 11, f: 12, r: 13, '"': 34, '\\': 92 };

/** A path as git prints it: plain, or C-quoted (`"a/caf\303\251"`) when it holds unusual bytes. */
export function unquote(s: string): string {
  if (s.length < 2 || !s.startsWith('"') || !s.endsWith('"')) return s;
  const bytes: number[] = [];
  const body = s.slice(1, -1);
  for (let i = 0; i < body.length; i++) {
    const c = body[i] as string;
    if (c !== '\\') {
      for (const b of new TextEncoder().encode(c)) bytes.push(b);
      continue;
    }
    const n = body[i + 1] ?? '';
    if (/[0-7]/.test(n)) {
      bytes.push(parseInt(body.slice(i + 1, i + 4), 8) & 0xff);
      i += 3;
    } else {
      bytes.push(ESCAPES[n] ?? n.charCodeAt(0));
      i += 1;
    }
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}

// `--- a/x`, `+++ b/x` or `/dev/null`, with the tab git adds after a name that holds a space.
function headerPath(rest: string): string | undefined {
  const name = unquote(rest.replace(/\t$/, ''));
  if (name === '/dev/null') return undefined;
  return name.replace(/^[ab]\//, '');
}

// `diff --git a/x b/x`; quoted names come as `"a/..." "b/..."`. For a new file read through --no-index it is `a/dev/null b/x`.
function gitHeaderPath(rest: string): string {
  if (rest.startsWith('"')) {
    const m = /"(?:[^"\\]|\\.)*"\s*$/.exec(rest);
    if (m) return unquote(m[0].trim()).replace(/^b\//, '');
  }
  const at = rest.lastIndexOf(' b/');
  return at >= 0 ? rest.slice(at + 3) : rest;
}

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@ ?(.*)$/;

export function parseDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let cur: DiffFile | undefined;
  let hunk: DiffHunk | undefined;
  let oldNo = 0;
  let newNo = 0;
  let oldLeft = 0;
  let newLeft = 0;
  for (const line of text.split('\n')) {
    const raw = line.endsWith('\r') ? line.slice(0, -1) : line;
    if (raw.startsWith('diff --git ')) {
      cur = { path: gitHeaderPath(raw.slice('diff --git '.length)), status: 'modified', binary: false, added: 0, removed: 0, lines: [], hunks: [] };
      files.push(cur);
      hunk = undefined;
      continue;
    }
    if (!cur) continue;
    const inHunk = hunk !== undefined && (oldLeft > 0 || newLeft > 0);
    if (raw.startsWith('@@')) {
      const m = HUNK.exec(raw);
      if (!m) continue;
      hunk = {
        oldStart: Number(m[1]), oldLines: m[2] === undefined ? 1 : Number(m[2]),
        newStart: Number(m[3]), newLines: m[4] === undefined ? 1 : Number(m[4]),
        section: m[5] ?? '', at: cur.lines.length,
      };
      cur.hunks.push(hunk);
      cur.lines.push({ kind: 'hunk', text: raw });
      oldNo = hunk.oldStart;
      newNo = hunk.newStart;
      oldLeft = hunk.oldLines;
      newLeft = hunk.newLines;
    } else if (inHunk && raw.startsWith('+')) {
      cur.added++;
      cur.lines.push({ kind: 'add', text: raw.slice(1), newNo: newNo++ });
      newLeft--;
    } else if (inHunk && raw.startsWith('-')) {
      cur.removed++;
      cur.lines.push({ kind: 'del', text: raw.slice(1), oldNo: oldNo++ });
      oldLeft--;
    } else if (inHunk && (raw.startsWith(' ') || raw === '')) {
      // An empty line inside a hunk is an unchanged empty line whose leading space was trimmed on the way.
      cur.lines.push({ kind: 'ctx', text: raw.slice(1), oldNo: oldNo++, newNo: newNo++ });
      oldLeft--;
      newLeft--;
    } else if (raw.startsWith('\\')) {
      const last = cur.lines[cur.lines.length - 1];
      if (last && last.kind !== 'hunk') last.noEol = true;
    } else if (hunk === undefined) {
      // File headers, before the first hunk.
      if (raw.startsWith('new file mode')) cur.status = 'added';
      else if (raw.startsWith('deleted file mode')) cur.status = 'deleted';
      else if (raw.startsWith('rename from ')) { cur.oldPath = unquote(raw.slice('rename from '.length)); cur.status = 'renamed'; }
      else if (raw.startsWith('rename to ')) { cur.path = unquote(raw.slice('rename to '.length)); cur.status = 'renamed'; }
      else if (raw.startsWith('Binary files ') || raw === 'GIT binary patch') cur.binary = true;
      else if (raw.startsWith('+++ ')) { const p = headerPath(raw.slice(4)); if (p !== undefined) cur.path = p; }
      else if (raw.startsWith('--- ')) {
        const p = headerPath(raw.slice(4));
        if (p === undefined) cur.status = 'added';
        else if (cur.status !== 'renamed') cur.path = p;
      }
    }
  }
  for (const f of files) {
    // `+++ /dev/null`: the file is gone, and its name came from the `---` line.
    if (f.status === 'modified' && f.hunks.length > 0 && f.lines.every((l) => l.kind === 'hunk' || l.kind === 'del')
      && f.hunks.every((h) => h.newLines === 0 && h.newStart === 0)) f.status = 'deleted';
  }
  if (isTruncated(text) && files.length > 0) (files[files.length - 1] as DiffFile).partial = true;
  return files;
}

/** One row of the side-by-side view: the old line on the left, the new one on the right, or a hunk heading across both. */
export interface SplitRow { left?: DiffLine; right?: DiffLine; hunk?: DiffLine }

/** Unchanged lines on both sides; within a run of changes each deleted line faces the added line at the same position. */
export function splitRows(lines: DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let dels: DiffLine[] = [];
  let adds: DiffLine[] = [];
  const flush = () => {
    for (let i = 0; i < Math.max(dels.length, adds.length); i++) rows.push({ left: dels[i], right: adds[i] });
    dels = [];
    adds = [];
  };
  for (const l of lines) {
    if (l.kind === 'del') {
      if (adds.length > 0) flush();
      dels.push(l);
    } else if (l.kind === 'add') {
      adds.push(l);
    } else {
      flush();
      rows.push(l.kind === 'hunk' ? { hunk: l } : { left: l, right: l });
    }
  }
  flush();
  return rows;
}

/** For each line of a run of changes, the line it replaces or is replaced by (the same position in the facing run), if any. */
export function pairedLines(lines: DiffLine[]): Map<DiffLine, DiffLine> {
  const pairs = new Map<DiffLine, DiffLine>();
  for (const r of splitRows(lines)) {
    if (r.left && r.right && r.left !== r.right) {
      pairs.set(r.left, r.right);
      pairs.set(r.right, r.left);
    }
  }
  return pairs;
}

/** The part of `a` that differs from `b`, as [start, end) after their common start and common end; undefined when equal. */
export function changedSpan(a: string, b: string): [number, number] | undefined {
  if (a === b) return undefined;
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let end = 0;
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++;
  return [start, a.length - end];
}

export interface FolderGroup { folder: string; files: DiffFile[] }

/** Files by folder: the top folder first, then folders by name; files by name inside each. */
export function byFolder(files: DiffFile[]): FolderGroup[] {
  const groups = new Map<string, DiffFile[]>();
  for (const f of files) {
    const at = f.path.lastIndexOf('/');
    const folder = at >= 0 ? f.path.slice(0, at) : '';
    groups.set(folder, [...(groups.get(folder) ?? []), f]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b)))
    .map(([folder, list]) => ({ folder, files: [...list].sort((x, y) => baseName(x.path).localeCompare(baseName(y.path))) }));
}

export function baseName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

/** Unchanged lines of the new file that a diff leaves out: before hunk `before` (or after the last hunk when `before` equals the
 *  number of hunks, up to the end of the file: `to` is Infinity). `offset` turns a new line number into the old one. */
export interface Gap { before: number; from: number; to: number; offset: number }

// The last line of the new (or old) file a hunk covers; a hunk with no new lines sits after line `start`.
const endOf = (start: number, count: number) => (count === 0 ? start : start + count - 1);
const startOf = (start: number, count: number) => (count === 0 ? start + 1 : start);

export function gapsOf(file: DiffFile): Gap[] {
  if (file.binary || file.status === 'added' || file.status === 'deleted' || file.hunks.length === 0) return [];
  const gaps: Gap[] = [];
  let newEnd = 0;
  let oldEnd = 0;
  file.hunks.forEach((h, i) => {
    const from = newEnd + 1;
    const to = startOf(h.newStart, h.newLines) - 1;
    if (to >= from) gaps.push({ before: i, from, to, offset: oldEnd - newEnd });
    newEnd = endOf(h.newStart, h.newLines);
    oldEnd = endOf(h.oldStart, h.oldLines);
  });
  if (!file.partial) gaps.push({ before: file.hunks.length, from: newEnd + 1, to: Infinity, offset: oldEnd - newEnd });
  return gaps;
}

/** Does the file as read now still hold the new side of this diff where the diff says? If not, it changed after the diff was read. */
export function matchesFile(file: DiffFile, fileLines: string[]): boolean {
  return file.lines.every((l) => l.newNo === undefined || (l.kind !== 'add' && l.kind !== 'ctx') || fileLines[l.newNo - 1] === l.text);
}

/** The text of a file as lines, the way the diff numbers them. */
export function fileLines(text: string): string[] {
  const lines = text.split('\n').map((l) => (l.endsWith('\r') ? l.slice(0, -1) : l));
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
  return lines;
}

/** The unchanged lines of `gap`, numbered on both sides, from the current file's lines. */
export function gapLines(gap: Gap, lines: string[]): DiffLine[] {
  const out: DiffLine[] = [];
  for (let n = gap.from; n <= Math.min(gap.to, lines.length); n++) out.push({ kind: 'ctx', text: lines[n - 1] ?? '', newNo: n, oldNo: n + gap.offset });
  return out;
}

export interface ChangeCounts { all: number; added: number; modified: number; deleted: number; renamed: number }

export function countByStatus(files: DiffFile[]): ChangeCounts {
  const c: ChangeCounts = { all: files.length, added: 0, modified: 0, deleted: 0, renamed: 0 };
  for (const f of files) c[f.status]++;
  return c;
}

/** Is `b` the same change as `a` (the same lines, in the same order)? Used to tell whether a file read now still belongs to the
 *  diff on screen: a change inside a folded region leaves the diff's own lines in place but changes the file's diff. */
export function sameChange(a: DiffFile, b: DiffFile | undefined): boolean {
  return !!b && a.lines.length === b.lines.length && a.lines.every((l, i) => l.kind === b.lines[i]?.kind && l.text === b.lines[i]?.text);
}
