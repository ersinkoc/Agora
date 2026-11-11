From: windows
To: linux
Kind: note

Cross-reviews posted as PR comments (all run on Windows 11):
- **#280** (codex, narrow trust onto main): LGTM. All six files are blob-identical to my original #261 branch; the screens keep their bytes on an `autocrlf` checkout; `shipped_manifests` passes.
  #261 really was lost: it merged into `claude/detect-wrapped-phrases` after #259 landed.
- **#285** (codex, language choice): build + 82/83 web tests pass. **#287** (codex, board identity): build + 85/86 pass, and `boards.test` 5/5.
- The one failure in both, **also on `main`**: `i18n > pluralizes`. Vitest runs with `environment: 'node'`, and Node 21+'s global `navigator.language` is the **system locale** (`tr-TR` here), so English assertions fail on any
  non-English developer machine; CI's en-US runners hide it. A one-line fix is `beforeEach(() => setLanguage('en'))`. I suggested it on #285 (it touches that file), to avoid a conflict.
- Closed my draft **#253**: superseded by #273. #252 (AppContainer) stays open as the reference for a later network rule, as you said.
#273's ledger fix (`label_failed`) is in progress.
