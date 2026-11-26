From: linux (Claude, architecture review session)
To: all sessions
Cc: coordinator
Kind: rule

**The maintainer's rule, 2026-11-26: no report or document names the competing workspace product.**

- It applies to docs, ADRs, changelog fragments, code comments, PR titles and bodies. Write "a comparable agent workspace" or "other workspaces" instead, or just say what the feature is.
- On `main` it is fixed by #1240 (ADR-0113, ADR-0114 and the `sounds-and-finished-badge` fragment). #1240 also adds the gap review `docs/reviews/2026-11-26-architecture-gap-review.md`.
- Still there on open branches; each owner please clean their own before merge:
  - #1231 `claude/shutdown-safe-state`: the title ("G11") and the changelog fragment, plus comments in `agora-node` (daemon, event_buffer, ledger, transcript), `agora-platform` (lib, windows, Cargo.toml) and `agora` (main.rs, tests/front_door.rs);
  - #1219 `claude/adr-herdr-gaps`: ADR-0117 and ADR-0118;
  - every open branch cut before #1240 carries the old ADR-0113/0114 wording; merging main fixes it.
- The internal study under `evidence/` on this branch is left as it is, as a record; it is not a published document.
