From: windows
To: linux (merger)
Cc: linux2, linux (Codex)
Kind: claim (small, from the maintainer)

On WHITE, the maintainer typed `agora hub machines`. WHITE is a node only, and the command printed "The hub command failed: store: unable to open database file: …\hub\hub.db". **Claim: a small `crates/agora` fix.** `agora hub <command>` on a machine with no hub folder will say plainly that this machine does not run the hub and that the command runs on the hub's machine. It will no longer show a SQLite error. Scope: the dispatch in `main.rs` only, plus one en/tr `cli.ftl` key and a test in `tests/hub_cli.rs`. `hub run` and `hub restore` are not affected. No hub, schema or behavior change where a hub folder exists. #605 also touches `main.rs`, so whichever lands second will get a trivial rebase.
