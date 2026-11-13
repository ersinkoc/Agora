// Reads the unified diff a node returns (`git.diff`) into files and lines the screen can colour. Pure: text in, data out.

export interface DiffLine { kind: 'add' | 'del' | 'ctx' | 'hunk'; text: string }
export interface DiffFile { path: string; added: number; removed: number; lines: DiffLine[] }

// One file's change as a conversation shows it: the hunks of a diff with or without git's `diff --git` line (a chat agent's diff
// starts at `---`), counted. Headers are skipped; lines before the first `@@` are not part of the change.
export function parseHunks(text: string): Omit<DiffFile, 'path'> {
  const out: Omit<DiffFile, 'path'> = { added: 0, removed: 0, lines: [] };
  let inHunk = false;
  for (const raw of text.split('\n')) {
    if (raw.startsWith('@@')) { inHunk = true; out.lines.push({ kind: 'hunk', text: raw }); }
    else if (!inHunk) continue;
    else if (raw.startsWith('+')) { out.added++; out.lines.push({ kind: 'add', text: raw.slice(1) }); }
    else if (raw.startsWith('-')) { out.removed++; out.lines.push({ kind: 'del', text: raw.slice(1) }); }
    else if (raw.startsWith(' ')) out.lines.push({ kind: 'ctx', text: raw.slice(1) });
    else if (raw.startsWith('diff --git ')) inHunk = false;
  }
  return out;
}

export function parseDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let cur: DiffFile | undefined;
  let inHunk = false;
  for (const raw of text.split('\n')) {
    if (raw.startsWith('diff --git ')) {
      // `diff --git a/x b/x`; for a new file read through --no-index it is `a/dev/null b/x`: the last `b/` wins.
      const at = raw.lastIndexOf(' b/');
      cur = { path: at >= 0 ? raw.slice(at + 3) : raw.slice('diff --git '.length), added: 0, removed: 0, lines: [] };
      files.push(cur);
      inHunk = false;
    } else if (!cur) {
      continue;
    } else if (raw.startsWith('@@')) {
      inHunk = true;
      cur.lines.push({ kind: 'hunk', text: raw });
    } else if (!inHunk) {
      continue; // file headers: index, ---, +++, mode lines
    } else if (raw.startsWith('+')) {
      cur.added++;
      cur.lines.push({ kind: 'add', text: raw.slice(1) });
    } else if (raw.startsWith('-')) {
      cur.removed++;
      cur.lines.push({ kind: 'del', text: raw.slice(1) });
    } else if (raw.startsWith(' ')) {
      cur.lines.push({ kind: 'ctx', text: raw.slice(1) });
    }
  }
  return files;
}
