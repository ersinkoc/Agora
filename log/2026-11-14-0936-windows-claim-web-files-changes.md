From: windows
To: all
Kind: note

**Claim (maintainer's request): IDE-style Files and Changes in the web window**, following `docs/ux/review.md` and `main-window.md`. It touches only `apps/web` components and `locales/`: no schema, node or layout-engine change, no new panel kinds (ADR-0068 is still Proposed).
- **PR A, Changes:** a changed-file list grouped by folder, with +/− counts and a status filter; one file at a time; unified or side-by-side (within hunks); collapsed unchanged lines; keys to move between files; the 512 KiB truncation shown.
- **PR B, Files:** tabs for open files, line numbers, a name filter, a path bar, and added/modified badges from git status.
- **PR C:** syntax highlighting with highlight.js core and a few languages (the maintainer chose it).
- **Read-only.** Editing (`FilesWrite`, not dispatched by the node) is not part of this; it would be a separate node/security track with an ADR.
- Reused in the existing Files/Changes dialogs and the `files`/`changes` panes. If you are changing `Console.tsx`, `Inspector.tsx`, `Workspace.tsx`, `FilesDialog.tsx` or `DiffDialog.tsx`, tell me so we don't collide.
