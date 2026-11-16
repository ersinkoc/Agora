# Changes-view review (#483)

Head: `e15b0adf2d759446a8b31135ab1f65ddf5e73ad6`. Result: changes requested.

The browser proof mounts the actual `ChangesView` with controlled `load` and `read` callbacks. It uses
Chromium at 1100x820, in English and Turkish. No hub command, real agent, login or paid request is involved.
`0483-changes-review.json` records all eight runs. The two screenshots show the successful normal case
in both themes, not a simulation of the failures.

| Case | Observed in both languages |
| --- | --- |
| Normal `safe.txt`, Whole file | Current context remains visible; no errors |
| Root file `constructor` | `Cannot read properties of undefined (reading 'includes')`; root unmounted |
| 170,000 short added lines, 510,107 bytes | `Maximum call stack size exceeded`; root unmounted despite the 3,000-row draw budget |
| Old read fails after refresh and a successful current read | Valid current context disappears; old failure replaces it with Could not read this file |

The first two failures come from an inherited per-file object entry and a spread into `Math.max` before
the draw budget is applied. They corroborate the existing review threads at `ChangesView.tsx:90` and
`:303`. The refresh failure corroborates `:123`: open Whole file with read A pending, refresh, open Whole
file with read B returning the current contents, then reject A. Both old success and failure handlers
must be excluded from the current generation.

The large case uses this valid diff (510,107 UTF-8 bytes, below the node's 524,288-byte cap):

```js
const diff = 'diff --git a/large.txt b/large.txt\nnew file mode 100644\n'
  + '--- /dev/null\n+++ b/large.txt\n@@ -0,0 +1,170000 @@\n'
  + '+x\n'.repeat(170000);
```

An additional parser proof uses an actual local Git repository with a file named `emoji"😀.txt` and
`git -c core.quotePath=false diff`. The exact output is `0483-unicode-path.diff`. `parseDiff` returns
`emoji"��.txt`: encoding each UTF-16 code unit separately replaces the emoji's surrogate halves.
Encoding complete code points or unescaped spans must preserve the existing C/octal escape handling.

The baseline TypeScript check, all 330 tracked tests and production web build pass after removing the
temporary proofs. The real-Git Unicode assertion fails. No product source changes were retained, and
the private preview process was stopped. The findings are posted on #483; dependent #487/#488/#500
cannot receive an end-to-end approval until these corrections land.
