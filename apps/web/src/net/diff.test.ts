import { describe, expect, it } from 'vitest';
import {
  byFolder, changedSpan, countByStatus, fileLines, gapLines, gapsOf, isTruncated, MAX_DIFF_BYTES, matchesFile, pairedLines, parseDiff,
  lineEndingOnly, parseHunks, sameChange, splitRows, unquote,
} from './diff';

const SAMPLE = `diff --git a/a.txt b/a.txt
index 1..2 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
 keep
-old
+new
diff --git a/dev/null b/fresh.txt
new file mode 100644
--- /dev/null
+++ b/fresh.txt
@@ -0,0 +1 @@
+brand new
`;

// Two hunks in a 20-line file: line 3 changed, line 15 removed and two lines added after it.
const TWO_HUNKS = `diff --git a/src/app.ts b/src/app.ts
index 3..4 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,6 +1,6 @@ export function main() {
 l1
 l2
-l3
+L3
 l4
 l5
 l6
@@ -12,7 +12,8 @@ function tail() {
 l12
 l13
 l14
-l15
+n15a
+n15b
 l16
 l17
 l18
`;

describe('parseDiff', () => {
  it('splits files, counts lines and ignores headers', () => {
    const f = parseDiff(SAMPLE);
    expect(f.map((x) => [x.path, x.added, x.removed, x.status])).toEqual([['a.txt', 1, 1, 'modified'], ['fresh.txt', 1, 0, 'added']]);
    expect(f[0]?.lines.map((l) => l.kind)).toEqual(['hunk', 'ctx', 'del', 'add']);
    expect(f[1]?.lines[1]).toMatchObject({ kind: 'add', text: 'brand new', newNo: 1 });
  });

  it('is empty for no diff', () => expect(parseDiff('')).toEqual([]));

  it('numbers lines on both sides and keeps the hunk section', () => {
    const [f] = parseDiff(TWO_HUNKS);
    expect(f?.hunks.map((h) => [h.oldStart, h.oldLines, h.newStart, h.newLines, h.section])).toEqual([
      [1, 6, 1, 6, 'export function main() {'], [12, 7, 12, 8, 'function tail() {'],
    ]);
    const nums = f?.lines.filter((l) => l.kind !== 'hunk').map((l) => `${l.kind}:${l.oldNo ?? '-'}:${l.newNo ?? '-'}`);
    expect(nums?.slice(0, 5)).toEqual(['ctx:1:1', 'ctx:2:2', 'del:3:-', 'add:-:3', 'ctx:4:4']);
    expect(nums?.slice(-6)).toEqual(['del:15:-', 'add:-:15', 'add:-:16', 'ctx:16:17', 'ctx:17:18', 'ctx:18:19']);
  });

  it('reads deleted, renamed and binary files, quoted names and CRLF', () => {
    const text = [
      'diff --git a/gone.txt b/gone.txt', 'deleted file mode 100644', '--- a/gone.txt', '+++ /dev/null', '@@ -1 +0,0 @@', '-bye',
      'diff --git a/old name.md b/new name.md', 'similarity index 90%', 'rename from old name.md', 'rename to new name.md',
      'diff --git a/logo.png b/logo.png', 'Binary files a/logo.png and b/logo.png differ',
      'diff --git "a/caf\\303\\251.txt" "b/caf\\303\\251.txt"', '--- "a/caf\\303\\251.txt"', '+++ "b/caf\\303\\251.txt"', '@@ -1 +1 @@', '-a\r', '+b\r',
    ].join('\n');
    const f = parseDiff(text);
    expect(f.map((x) => [x.path, x.status, x.binary])).toEqual([
      ['gone.txt', 'deleted', false], ['new name.md', 'renamed', false], ['logo.png', 'modified', true], ['café.txt', 'modified', false],
    ]);
    expect(f[1]?.oldPath).toBe('old name.md');
    expect(f[3]?.lines.map((l) => l.text)).toEqual(['@@ -1 +1 @@', 'a', 'b']);
  });

  // What a node answers for a workspace that is the folder `web/` of a repository (captured from agora-node's
  // `a_folder_workspace_names_every_file_from_the_folder`): every name is from the folder, as `git.diff` and `files.read` take them.
  it('reads the names of a folder workspace as paths inside the folder', () => {
    const files = parseDiff(`diff --git a/img.bin b/img.bin
index 8352675..797c34e 100644
Binary files a/img.bin and b/img.bin differ
diff --git a/in b/in.txt b/in b/in.txt
new file mode 100644
index 0000000..4935e88
--- /dev/null
+++ b/in b/in.txt\t
@@ -0,0 +1 @@
+in
diff --git a/m.txt b/m.txt
deleted file mode 100644
index d3f59cb..0000000
--- a/m.txt
+++ /dev/null
@@ -1 +0,0 @@
-move me
diff --git a/old.txt b/renamed.txt
similarity index 100%
rename from old.txt
rename to renamed.txt
diff --git a/w.txt b/w.txt
index c077218..309e238 100644
--- a/w.txt
+++ b/w.txt
@@ -1 +1,2 @@
 web
+INSIDE
diff --git a/x b/f.txt b/x b/f.txt
index 6a69f92..ab71375 100644
--- a/x b/f.txt\t
+++ b/x b/f.txt\t
@@ -1 +1,2 @@
 f
+g
diff --git a/new.bin b/new.bin
new file mode 100644
index 0000000..00ffac9
Binary files /dev/null and b/new.bin differ
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..3e75765
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+new
`);
    expect(files.map((f) => [f.path, f.status, f.binary])).toEqual([
      ['img.bin', 'modified', true],
      ['in b/in.txt', 'added', false],
      ['m.txt', 'deleted', false],
      ['renamed.txt', 'renamed', false],
      ['w.txt', 'modified', false],
      ['x b/f.txt', 'modified', false],
      ['new.bin', 'added', true],
      ['new.txt', 'added', false],
    ]);
    expect(files[3]?.oldPath).toBe('old.txt');
  });

  it('marks a missing newline at the end', () => {
    const [f] = parseDiff('diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+a\n');
    expect(f?.lines[1]).toMatchObject({ kind: 'del', noEol: true });
    expect(f?.lines[2]?.noEol).toBeUndefined();
  });

  it('marks the last file of a diff the node cut off', () => {
    const big = `diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,1 +1,99999 @@\n${'+0123456789\n'.repeat(MAX_DIFF_BYTES / 12 + 1)}`.slice(0, MAX_DIFF_BYTES);
    expect(isTruncated(big)).toBe(true);
    expect(isTruncated(SAMPLE)).toBe(false);
    expect(parseDiff(big)[0]?.partial).toBe(true);
    expect(gapsOf(parseDiff(big)[0] as never).some((g) => g.to === Infinity)).toBe(false);
  });
});

describe('unquote', () => {
  it('decodes git C-quoted names', () => {
    expect(unquote('plain/name.txt')).toBe('plain/name.txt');
    expect(unquote('"a/\\303\\274ber \\"q\\" tab\\t.txt"')).toBe('a/über "q" tab\t.txt');
  });

  it('keeps a literal character outside the BMP whole (core.quotePath=false)', () => {
    expect(unquote('"a/emoji\\"😀.txt"')).toBe('a/emoji"😀.txt');
    expect(unquote('"😀\\360\\237\\230\\200"')).toBe('😀😀');
  });
});

describe('line endings', () => {
  it('counts lines that changed only from LF to CR LF (real git output)', () => {
    const [f] = parseDiff('diff --git a/f.txt b/f.txt\nindex 4cb29ea..e08aa17 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n'
      + '-one\n-two\n-three\n+one\r\n+two\r\n+THREE\r\n');
    expect(f?.lines.filter((l) => l.kind === 'add').map((l) => [l.text, l.cr])).toEqual([['one', true], ['two', true], ['THREE', true]]);
    expect(lineEndingOnly(f?.lines ?? [])).toBe(2);
  });
});

describe('a folder whose name holds " b/"', () => {
  it('names empty new, binary and mode-only files right (real git output)', () => {
    const files = parseDiff([
      'diff --git a/dir b/empty.txt b/dir b/empty.txt', 'new file mode 100644', 'index 0000000..e69de29',
      'diff --git a/dir b/file.bin b/dir b/file.bin', 'new file mode 100644', 'index 0000000..8352675',
      'Binary files /dev/null and b/dir b/file.bin differ',
      'diff --git a/dir b/mode.sh b/dir b/mode.sh', 'old mode 100644', 'new mode 100755', '',
    ].join('\n'));
    expect(files.map((f) => [f.path, f.status, f.binary])).toEqual([
      ['dir b/empty.txt', 'added', false], ['dir b/file.bin', 'added', true], ['dir b/mode.sh', 'modified', false],
    ]);
  });

  it('takes a binary file\'s name from the header when the name holds " and " (Binary files … and … cannot be split)', () => {
    const files = parseDiff([
      'diff --git a/q and b/m.bin b/q and b/m.bin', 'index 1111111..2222222 100644', 'Binary files a/q and b/m.bin and b/q and b/m.bin differ',
      'diff --git a/n and b/new.bin b/n and b/new.bin', 'new file mode 100644', 'index 0000000..3333333',
      'Binary files /dev/null and b/n and b/new.bin differ',
      'diff --git a/Terms and Conditions.pdf b/Terms and Conditions.pdf', 'new file mode 100644', 'index 0000000..4444444',
      'Binary files /dev/null and b/Terms and Conditions.pdf differ', '',
    ].join('\n'));
    expect(files.map((f) => [f.path, f.status, f.binary])).toEqual([
      ['q and b/m.bin', 'modified', true], ['n and b/new.bin', 'added', true], ['Terms and Conditions.pdf', 'added', true],
    ]);
  });

  it('still reads the --no-index form and a deleted binary file', () => {
    const files = parseDiff([
      'diff --git a/dev/null b/fresh.bin', 'new file mode 100644', 'Binary files /dev/null and b/fresh.bin differ',
      'diff --git a/x b/gone.bin b/x b/gone.bin', 'deleted file mode 100644', 'Binary files a/x b/gone.bin and /dev/null differ', '',
    ].join('\n'));
    expect(files.map((f) => [f.path, f.status])).toEqual([['fresh.bin', 'added'], ['x b/gone.bin', 'deleted']]);
  });
});

describe('deleted or emptied', () => {
  it('calls a file deleted only for `+++ /dev/null`, not one emptied to nothing', () => {
    const files = parseDiff([
      'diff --git a/empty.txt b/empty.txt', '--- a/empty.txt', '+++ b/empty.txt', '@@ -1,2 +0,0 @@', '-one', '-two',
      'diff --git a/gone.txt b/gone.txt', '--- a/gone.txt', '+++ /dev/null', '@@ -1 +0,0 @@', '-bye', '',
    ].join('\n'));
    expect(files.map((f) => [f.path, f.status])).toEqual([['empty.txt', 'modified'], ['gone.txt', 'deleted']]);
  });

  it('reads the real git diff of a quoted emoji name', () => {
    const [f] = parseDiff([
      'diff --git "a/emoji\\"😀.txt" "b/emoji\\"😀.txt"', 'new file mode 100644', 'index 0000000..ce01362', '--- /dev/null',
      '+++ "b/emoji\\"😀.txt"', '@@ -0,0 +1 @@', '+hello', '',
    ].join('\n'));
    expect(f?.path).toBe('emoji"😀.txt');
  });
});

describe('side by side', () => {
  it('pairs each deleted line with the added line at the same place', () => {
    const [f] = parseDiff(TWO_HUNKS);
    const rows = splitRows(f?.lines ?? []);
    const shown = rows.map((r) => (r.hunk ? 'H' : `${r.left?.text ?? '·'}|${r.right?.text ?? '·'}`));
    expect(shown.slice(0, 5)).toEqual(['H', 'l1|l1', 'l2|l2', 'l3|L3', 'l4|l4']);
    expect(shown.slice(-5)).toEqual(['l15|n15a', '·|n15b', 'l16|l16', 'l17|l17', 'l18|l18']);
    const pairs = pairedLines(f?.lines ?? []);
    const l3 = f?.lines.find((l) => l.text === 'l3');
    expect(pairs.get(l3 as never)?.text).toBe('L3');
  });

  it('finds the changed part of a changed line', () => {
    expect(changedSpan('const a = 1;', 'const a = 22;')).toEqual([10, 11]);
    expect(changedSpan('same', 'same')).toBeUndefined();
    expect(changedSpan('ab', 'abc')).toEqual([2, 2]);
  });
});

describe('unchanged lines between hunks', () => {
  it('knows where they are and numbers them on both sides', () => {
    const [f] = parseDiff(TWO_HUNKS);
    const gaps = gapsOf(f as never);
    expect(gaps.map((g) => [g.before, g.from, g.to, g.offset])).toEqual([[1, 7, 11, 0], [2, 20, Infinity, -1]]);
    const now = Array.from({ length: 20 }, (_, i) => `l${i + 1}`);
    now.splice(2, 1, 'L3');
    now.splice(14, 1, 'n15a', 'n15b');
    expect(matchesFile(f as never, now)).toBe(true);
    expect(gapLines(gaps[0] as never, now).map((l) => `${l.oldNo}:${l.newNo}:${l.text}`)).toEqual(['7:7:l7', '8:8:l8', '9:9:l9', '10:10:l10', '11:11:l11']);
    expect(gapLines(gaps[1] as never, now).map((l) => `${l.oldNo}:${l.newNo}:${l.text}`)).toEqual(['19:20:l19', '20:21:l20']);
    expect(matchesFile(f as never, now.map((l) => (l === 'l13' ? 'edited' : l)))).toBe(false);
  });

  it('tells a change inside a folded region from no change', () => {
    const [f] = parseDiff(TWO_HUNKS);
    expect(sameChange(f as never, parseDiff(TWO_HUNKS)[0])).toBe(true);
    expect(sameChange(f as never, parseDiff(TWO_HUNKS.replace('+L3', '+L3!'))[0])).toBe(false);
    expect(sameChange(f as never, undefined)).toBe(false);
  });

  it('reads file text the way the diff numbers it', () => {
    expect(fileLines('a\r\nb\n')).toEqual(['a', 'b']);
    expect(fileLines('a\nb')).toEqual(['a', 'b']);
  });

  it('has none for an added file', () => expect(gapsOf(parseDiff(SAMPLE)[1] as never)).toEqual([]));
});

describe('the file list', () => {
  it('groups by folder, top folder first, and counts by status', () => {
    const files = parseDiff(`${TWO_HUNKS}${SAMPLE}diff --git a/src/z/b.ts b/src/z/b.ts\n--- a/src/z/b.ts\n+++ b/src/z/b.ts\n@@ -1 +1 @@\n-x\n+y\n`);
    expect(byFolder(files).map((g) => [g.folder, g.files.map((f) => f.path)])).toEqual([
      ['', ['a.txt', 'fresh.txt']], ['src', ['src/app.ts']], ['src/z', ['src/z/b.ts']],
    ]);
    expect(countByStatus(files)).toEqual({ all: 4, added: 1, modified: 3, deleted: 0, renamed: 0 });
  });
});

describe('parseHunks', () => {
  it('reads a chat agent\'s diff, which has no `diff --git` line', () => {
    // As the ACP bridge writes it (crates/agora-adapters/src/acp.rs `replace_diff`).
    const h = parseHunks('--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,2 @@\n-old\n+new\n+more\n');
    expect([h.added, h.removed]).toEqual([2, 1]);
    expect(h.lines.map((l) => l.kind)).toEqual(['hunk', 'del', 'add', 'add']);
  });
  it('numbers the lines from the hunk header: old numbers for deleted, new for added, both for unchanged', () => {
    const h = parseHunks('@@ -39,3 +39,3 @@ fn f\n keep\n-old\n+new\n tail\n');
    expect(h.lines.map((l) => [l.kind, l.oldNo, l.newNo])).toEqual([['hunk', undefined, undefined], ['ctx', 39, 39], ['del', 40, undefined], ['add', undefined, 40], ['ctx', 41, 41]]);
    expect(parseHunks('@@ odd @@\n+a\n').lines[1]?.newNo).toBeUndefined();
  });
  it('reads git\'s output too, and nothing without a hunk', () => {
    expect(parseHunks(SAMPLE).added).toBe(2);
    expect(parseHunks('just text')).toEqual({ added: 0, removed: 0, lines: [] });
  });
});
