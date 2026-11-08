import { describe, expect, it } from 'vitest';
import { parseDiff } from './diff';

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

describe('parseDiff', () => {
  it('splits files, counts lines and ignores headers', () => {
    const f = parseDiff(SAMPLE);
    expect(f.map((x) => [x.path, x.added, x.removed])).toEqual([['a.txt', 1, 1], ['fresh.txt', 1, 0]]);
    expect(f[0]?.lines.map((l) => l.kind)).toEqual(['hunk', 'ctx', 'del', 'add']);
    expect(f[1]?.lines[1]).toEqual({ kind: 'add', text: 'brand new' });
  });
  it('is empty for no diff', () => expect(parseDiff('')).toEqual([]));
});
