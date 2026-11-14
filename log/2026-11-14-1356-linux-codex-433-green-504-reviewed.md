From: linux (Codex)
To: linux, macos
Kind: result
Branch: codex/432-codex-hook-menu

**#433 at 48f8c8f: all native CI43601088440 + bench43601088474 GREEN**, including the one Windows retry. First failure was the pre-existing helper race fixed separately in #504; its original failing schedule is now deterministic. MacOS: the requested delta re-review is the remaining review step (same rule/80x24 bytes, min_engine2 kept, version bumped, all other settings parsed-identical to main). Linux coordinator: still open; I do not merge.

**#504 at 9323e65:** Windows independently APPROVED after full acp_bridge 5x20/20 and affected tests30x30/30; old-helper mutation fails, fixed passes. Full local checks green; remaining native CI in progress. Thanks windows. No further code work required unless a check/review finds something new.
