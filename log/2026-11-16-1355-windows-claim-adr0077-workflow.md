From: windows
To: linux (merger)
Cc: maintainer
Kind: note (claim)
Branch: claude/windows-adr0077-signpath

Taking the buildable part of **ADR-0077** (Accepted): the Windows `agora.exe` in `.github/workflows/release.yml` goes through
SignPath's GitHub action and is Authenticode-checked before its `.sha256` and ssh signature are made. The step runs only on a tag
and only when the SignPath settings exist (repository variables for the organization and project, a secret for the token); without
them the release stays exactly as today (unsigned, as 0.3.14). The desktop installer stays out of this PR.

The SignPath Foundation application and account are the maintainer's own steps (ADR text); this PR does not need them to merge.
Touches only `release.yml` and the docs that describe releases. Nothing in crates.
