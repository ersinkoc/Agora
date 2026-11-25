import { describe, expect, it } from 'vitest';
import { parseDiff } from './diff';
import { ancestors, changedFolders, changeMarks, crumbs, findMatches, statusByPath } from './fileview';

const DIFF = `diff --git a/src/a.ts b/src/a.ts
--- a/src/a.ts
+++ b/src/a.ts
@@ -1,5 +1,6 @@
 one
-two
+TWO
 three
+three and a half
 four
 five
@@ -10,4 +11,3 @@
 ten
-eleven
 twelve
 thirteen
@@ -20,2 +20,1 @@
 twenty
-twenty-one
diff --git a/README.md b/README.md
new file mode 100644
--- /dev/null
+++ b/README.md
@@ -0,0 +1 @@
+hello
`;

describe('the gutter', () => {
  it('marks changed, added and removed lines on the file as it is now', () => {
    const [f] = parseDiff(DIFF);
    expect([...changeMarks(f as never)].sort(([a], [b]) => a - b)).toEqual([[2, 'mod'], [4, 'add'], [12, 'del'], [21, 'del']]);
  });

  it('marks every line of a new file as added', () => {
    expect([...changeMarks(parseDiff(DIFF)[1] as never)]).toEqual([[1, 'add']]);
  });
});

describe('the tree', () => {
  it('knows each changed file and every folder above one', () => {
    const files = parseDiff(DIFF);
    expect([...statusByPath(files)]).toEqual([['src/a.ts', 'modified'], ['README.md', 'added']]);
    expect([...changedFolders(['a/b/c.txt', 'a/d.txt', 'top.txt'])].sort()).toEqual(['a', 'a/b']);
  });

  it('splits a path into the folders to open and the path bar', () => {
    expect(crumbs('src/net/files.ts')).toEqual([{ name: 'src', path: 'src' }, { name: 'net', path: 'src/net' }, { name: 'files.ts', path: 'src/net/files.ts' }]);
    expect(ancestors('src/net/files.ts')).toEqual(['src', 'src/net']);
    expect(ancestors('top.txt')).toEqual([]);
  });
});

describe('search in a file', () => {
  const lines = ['Alpha beta', 'beta BETA', '', 'gamma'];
  it('ignores case unless the query has a capital letter', () => {
    expect(findMatches(lines, 'beta')).toEqual([{ line: 0, start: 6, end: 10 }, { line: 1, start: 0, end: 4 }, { line: 1, start: 5, end: 9 }]);
    expect(findMatches(lines, 'BETA')).toEqual([{ line: 1, start: 5, end: 9 }]);
    // `İ` lowercases to two code units: the marks still fall on the line as it is.
    expect(findMatches(['İİabc İz'], 'abc')).toEqual([{ line: 0, start: 2, end: 5 }]);
    expect(findMatches(['a.b axb'], 'a.b')).toEqual([{ line: 0, start: 0, end: 3 }]);
    // Turkish: `i` finds `İ` and `I`, `ı` finds `I`; offsets stay on the line as it is.
    expect(findMatches(['İstanbul Izmir iç'], 'i').map((m) => m.start)).toEqual([0, 9, 12, 15]);
    expect(findMatches(['Işık ılık'], 'ı').map((m) => m.start)).toEqual([0, 2, 5, 7]);
    // Unicode simple-case peers which are not direct lower/upper-case conversions still match.
    expect(findMatches(['ẞ ς K'], 'ß')).toEqual([{ line: 0, start: 0, end: 1 }]);
    expect(findMatches(['ẞ ς K'], 'σ')).toEqual([{ line: 0, start: 2, end: 3 }]);
    expect(findMatches(['ẞ ς K'], 'k')).toEqual([{ line: 0, start: 4, end: 5 }]);
    expect(findMatches(['[a] (b)'], '[a]')).toEqual([{ line: 0, start: 0, end: 3 }]);
  });
  it('finds any punctuation as itself, a hyphen too, ignoring case or not', () => {
    expect(findMatches(['run --watch here'], '--watch')).toEqual([{ line: 0, start: 4, end: 11 }]);
    expect(findMatches(['x Foo-Bar y'], 'Foo-Bar')).toEqual([{ line: 0, start: 2, end: 9 }]);
    expect(findMatches(['a -> b'], '->')).toEqual([{ line: 0, start: 2, end: 4 }]);
    for (const c of '!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~') {
      expect(findMatches([`x${c}y ${c}`], c)).toEqual([{ line: 0, start: 1, end: 2 }, { line: 0, start: 4, end: 5 }]);
      expect(findMatches([`a${c}B A${c}B`], `A${c}B`)).toEqual([{ line: 0, start: 4, end: 7 }]);
    }
  });
  it('finds nothing for an empty query and stops at the limit', () => {
    expect(findMatches(lines, '')).toEqual([]);
    expect(findMatches(['aaaa'], 'a', 2)).toHaveLength(2);
  });
});
