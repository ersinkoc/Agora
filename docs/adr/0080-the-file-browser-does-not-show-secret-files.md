# 0080. The file browser does not show secret files
Status: Accepted
Date: 2026-11-15
Accepted: 2026-11-15 (the maintainer's explicit message in the merger chat, accepting the ADRs listed by number (#613 ADR-0060 Amendment A, #614 ADR-0078, #615 ADR-0080, #616 ADR-0079): they accepted all of them). The open questions' proposed answers stand.
Built: the node (#636) lists, refuses and counts as described. The web window and `agora tui` (this change) mark a secret entry with a lock and the reason "Kept private: it may hold a secret" (en, tr), do not send a read for it (Enter says the reason), show a changed secret file in the review as "Changed, content hidden" with the node's counts and no body, and show the same reason for a `files.secret` answer to a read (a client with an older listing). The phone layout shares these views.
Origin: a question the maintainer was asked while the file and review screens (#483, #487, #488, #500) were built: `.env` files and private keys show in
full in the file browser, which is the node's old behaviour. Asked "should they be hidden?", they approved, without
seeing details; this text is the detailed proposal and is not accepted until they read it and say so.

## Context
- `files.read` is confined to the worktree (lexically and after resolving links), bounded in size, and `files.list` hides `.git`. Nothing
  looks at a file's *name*: a worktree's `.env`, a private key someone left in the project, an `.npmrc` with a registry token are listed and read like any
  source file, by every window that may look at the project (the owner, a member given the project, a viewer).
- `git.diff` names every changed file and shows its content (ADR-0071). A changed `.env` therefore shows its new values in the review screen.
- The owner of the machine can read these files anyway. The people the rule protects are the **members and viewers** of a team (ADR-0049) and
  anyone a window is shared with, and the **hub**, which stores events: an event or a screenshot that carries a secret is kept.
- This is a convenience rule, not a sandbox: an agent in the worktree can still read these files, as the project's own code can. It decides what a
  *window* shows.

## Decision (proposed)
1. **The node decides, by name, what is secret.** A file is *secret* when its name matches this list, in any folder below the worktree, case-insensitively
   on Windows and macOS: `.env`, `.env.*` (except `.env.example`, `.env.sample`, `.env.template`, which are meant to be shared), `*.pem`, `*.key`, `*.p12`,
   `*.pfx`, `*.kdbx`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519` (not their `.pub`), `.npmrc`, `.pypirc`, `.netrc`, `.git-credentials`; and anything inside a
   folder named `.ssh`, `.aws` or `.gnupg`, or a file named `config.json` inside `.docker`.
2. **`files.list` still lists them**, marked as secret (a new boolean `secret` on the entry, an added field: old clients ignore it), so a person sees
   that the file exists and why it cannot be opened. It does not hide them: a missing `.env` would look like a bug.
3. **`files.read` of a secret file is refused** with a new stable error code `files.secret` (`path`). The node refuses; the hub does not decide.
4. **`git.diff` names a secret file and does not show its content**: the entry says it changed and carries the same `secret` marker, and no hunk.
   The change counts (added, removed) stay, since they hold no value.
5. **The owner can switch this off on the machine**, in the node's policy: `files.show_secret = true`. It is the owner's own file, on the owner's
   machine. The hub cannot change it (the hub cannot widen node policy), and `agora doctor` says when it is on.
6. **Not covered:** the content of an ordinary file that happens to hold a secret (a token pasted into `config.yml`), and what a terminal or an agent's chat
   shows. Scanning content for secrets is a different feature with false positives; this rule is by name only.

## Consequences
- A member or a viewer opening a project never sees a `.env` value or a private key through the file browser or the review screen.
- Protocol: one added field on a listing entry and one added error code (append-only); en and tr texts for the code; `protocol.md` and
  `error-codes.md` updated. Old clients show the code's text without the marker.
- Cost: a name check per listed entry and per read: no I/O, not a hot path.
- Someone who really wants to share a `.env` through a window turns the switch on, knowingly.

## Amendment (issue #847): writes
This decision covers reads and listings. `Files::write` is dormant and `files.write` stays disabled; before it is exposed the helper applies the same name policy to the requested path, every folder prefix and the real folder, and writes only through verified folder handles (`docs/security/threat-model.md` T53). No protocol or support-status change.

## Open questions (for the maintainer)
1. Is the name list right? Proposed: as written; a name is added later by an ADR amendment, never from the hub.
2. Should the owner's own windows also see the marker and be refused by default? Proposed: yes, one rule for everyone with one switch for the machine,
   so a shared screen or a recording never shows a secret by accident.
