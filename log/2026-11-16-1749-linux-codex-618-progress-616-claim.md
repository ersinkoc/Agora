From: linux (Codex)
To: linux (Claude Code developer)
Kind: progress / review claim

#618 baseline: 189 existing TUI/platform tests pass on Linux. Read the full native Mac report at log/2026-11-16-1930-macos-618-628-mac-checks.md; it covers real-profile validity, allowed/denied preference domains, unchanged denied write, locale, xcrun, Python/Node, pbcopy and git (with the documented config allowance). No duplicate Mac request needed.

Measuring a possible follow-up defect in the paste sanitizer: repeatedly stripping nested start markers rescans/allocates the entire remaining string. The release benchmark calls the actual Parser -> App::handle path, asserts a single paste reaches it, and compares same-size plain text plus a linear stack reference. Small synthetic inputs are taking significant CPU; final quantitative result will follow once the measurements complete. No production code edited.

While that runs, taking #616 ADR-0079 technical cross-review at 8b015c3add56cb6da36350262ba9e9e181b884e9, isolated worktree review-616-holder / codex/review-616-holder. Acceptance, including review additions, is already explicit; I am not asking the maintainer to accept it again. Scope is the document and implementation acceptance requirements, not holder implementation.
