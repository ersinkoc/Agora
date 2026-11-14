import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { parseDiff } from '../net/diff';
import { segmentsOf, truncatedText } from './ChangesView';

const DIFF = `diff --git a/src/app.ts b/src/app.ts
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,4 +1,4 @@
 l1
 l2
-l3
+L3
 l4
@@ -9,3 +9,3 @@
 l9
-l10
+L10
 l11
`;
const NOW = ['l1', 'l2', 'L3', 'l4', 'l5', 'l6', 'l7', 'l8', 'l9', 'L10', 'l11', 'l12'];

describe('the changes view', () => {
  beforeEach(() => setLanguage('en'));

  it('has every status, filter and layout word', () => {
    for (const s of ['modified', 'added', 'deleted', 'renamed']) {
      for (const k of [`changes-status-${s}`, `changes-badge-${s}`, `changes-filter-${s}`]) expect(t(k), k).not.toBe(k);
    }
    for (const k of ['changes-filter-all', 'changes-layout-unified', 'changes-layout-split']) expect(t(k), k).not.toBe(k);
    expect(t('changes-gap', { count: 4 })).toBe('Show 4 unchanged lines');
    for (const m of ['add', 'mod', 'del']) expect(t(`files-mark-${m}`), m).not.toBe(`files-mark-${m}`);
  });

  it('opens a gap and a file larger than a call takes arguments', () => {
    const n = 170_000;
    const [f] = parseDiff(`diff --git a/big.txt b/big.txt\n--- a/big.txt\n+++ b/big.txt\n@@ -1,1 +1,1 @@\n-a\n+A\n@@ -${n},1 +${n},1 @@\n-b\n+B\n`);
    const now = Array.from({ length: n }, (_, i) => (i === 0 ? 'A' : i === n - 1 ? 'B' : 'x'));
    const lines = segmentsOf(f as never, 'all', now).flatMap((s) => (s.kind === 'lines' ? s.lines : []));
    expect(lines.length).toBeGreaterThanOrEqual(n);
  });

  it('says how many files a cut diff leaves out, never more than the node counts', () => {
    expect(truncatedText(2, 8)).toContain('at least 6 are not listed');
    expect(truncatedText(1, 2)).toContain('at least one is not listed');
    // The node counts an untracked folder as one path, so it can count fewer than the diff lists: no number is made up then.
    expect(truncatedText(5, 3)).toBe(t('changes-truncated'));
    expect(truncatedText(2, undefined)).toBe(t('changes-truncated'));
  });

  it('folds the unchanged lines between hunks until they are opened', () => {
    const [f] = parseDiff(DIFF);
    const folded = segmentsOf(f as never, new Set(), undefined);
    expect(folded.map((s) => (s.kind === 'gap' ? `gap ${s.gap.from}-${s.gap.to}` : `${s.lines.length} lines`))).toEqual(['6 lines', 'gap 5-8', '5 lines', 'gap 12-Infinity']);
    const opened = segmentsOf(f as never, new Set([1]), NOW);
    expect(opened.map((s) => (s.kind === 'gap' ? 'gap' : s.lines.map((l) => l.text).join(',')))).toEqual([
      '@@ -1,4 +1,4 @@,l1,l2,l3,L3,l4,l5,l6,l7,l8,@@ -9,3 +9,3 @@,l9,l10,L10,l11', 'gap',
    ]);
  });

  it('shows the whole file as one run of lines, without hunk headings', () => {
    const [f] = parseDiff(DIFF);
    const all = segmentsOf(f as never, 'all', NOW);
    expect(all).toHaveLength(1);
    const lines = all[0]?.kind === 'lines' ? all[0].lines : [];
    expect(lines.map((l) => `${l.kind[0]}${l.oldNo ?? ''}/${l.newNo ?? ''}`)).toEqual([
      'c1/1', 'c2/2', 'd3/', 'a/3', 'c4/4', 'c5/5', 'c6/6', 'c7/7', 'c8/8', 'c9/9', 'd10/', 'a/10', 'c11/11', 'c12/12',
    ]);
  });

  it('drops the trailing fold when the file ends with the last hunk', () => {
    const [f] = parseDiff(DIFF);
    expect(segmentsOf(f as never, new Set([2]), NOW.slice(0, 11)).some((s) => s.kind === 'gap' && s.gap.to === Infinity)).toBe(false);
  });
});
