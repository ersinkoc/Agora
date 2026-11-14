// What the file browser shows next to a file's text, derived from data it already has: the change marks in the gutter (from the
// workspace diff), which files and folders hold changes, matches of a search, and the folders of a path. Pure: data in, data out.
import type { DiffFile, DiffStatus } from './diff';

/** A line's mark in the gutter: added, changed, or "lines were removed just above this line". */
export type ChangeMark = 'add' | 'mod' | 'del';

/** Marks by line number of the file as it is now (1-based). In each run of changes, added lines are "changed" when lines were removed
 *  just before them and "added" otherwise; a run that only removes marks the line after it (past the end: the viewer shows the
 *  mark on the last line). */
export function changeMarks(file: DiffFile): Map<number, ChangeMark> {
  const marks = new Map<number, ChangeMark>();
  const hunkAt = new Map(file.hunks.map((h) => [h.at, h]));
  const lines = file.lines;
  let lastNew = 0; // the last line of the new file seen so far
  let i = 0;
  while (i < lines.length) {
    const l = lines[i] as (typeof lines)[number];
    if (l.kind === 'hunk') {
      const h = hunkAt.get(i);
      if (h) lastNew = h.newLines === 0 ? h.newStart : h.newStart - 1;
      i++;
    } else if (l.kind === 'ctx') {
      lastNew = l.newNo ?? lastNew;
      i++;
    } else {
      let removed = 0;
      while (i < lines.length && lines[i]?.kind === 'del') { removed++; i++; }
      const added: number[] = [];
      while (i < lines.length && lines[i]?.kind === 'add') { added.push(lines[i]?.newNo ?? lastNew + 1); i++; }
      if (added.length > 0) {
        for (const n of added) marks.set(n, removed > 0 ? 'mod' : 'add');
        lastNew = added[added.length - 1] as number;
      } else if (!marks.has(lastNew + 1)) {
        marks.set(lastNew + 1, 'del');
      }
    }
  }
  return marks;
}

/** Each changed file's status, by its path in the workspace. */
export function statusByPath(files: DiffFile[]): Map<string, DiffStatus> {
  return new Map(files.map((f) => [f.path, f.status]));
}

/** Every folder that holds a changed file at any depth ('' is the top and is left out). */
export function changedFolders(paths: Iterable<string>): Set<string> {
  const out = new Set<string>();
  for (const p of paths) {
    let at = p.lastIndexOf('/');
    while (at > 0) {
      out.add(p.slice(0, at));
      at = p.lastIndexOf('/', at - 1);
    }
  }
  return out;
}

export interface Match { line: number; start: number; end: number }

/** Where `query` occurs in `lines` (0-based line index, [start, end) in the line), at most `limit` matches. Case is ignored unless
 *  the query holds a capital letter (as editors do). */
export function findMatches(lines: string[], query: string, limit = 10_000): Match[] {
  if (!query) return [];
  const exact = query !== query.toLowerCase();
  const q = exact ? query : query.toLowerCase();
  const out: Match[] = [];
  for (let i = 0; i < lines.length && out.length < limit; i++) {
    const text = exact ? (lines[i] as string) : (lines[i] as string).toLowerCase();
    let from = 0;
    for (;;) {
      const at = text.indexOf(q, from);
      if (at < 0 || out.length >= limit) break;
      out.push({ line: i, start: at, end: at + q.length });
      from = at + Math.max(1, q.length);
    }
  }
  return out;
}

export interface Crumb { name: string; path: string }

/** The folders of a path and the file itself, each with the path up to it. */
export function crumbs(path: string): Crumb[] {
  const parts = path.split('/').filter(Boolean);
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join('/') }));
}

/** The folders to open so that `path` shows in the tree. */
export function ancestors(path: string): string[] {
  return crumbs(path).slice(0, -1).map((c) => c.path);
}
