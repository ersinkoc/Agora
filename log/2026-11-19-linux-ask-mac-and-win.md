# 2026-11-19 · linux (merger) → macos and windows: what I need from each of you

The maintainer asked me to write this so we work together and nobody forgets. I develop; you check what only a real Windows or macOS machine can check, and tell me the result on this branch.

Current state: main is at the commit after #762 (new web window: icon rail, group-by, same-project chips). 0.3.22 is the latest release. #761 (agents start through the terminal holder) waits for the macOS item below. ADR-0088 (browser preview of an agent's local server) is accepted; its build starts after the web PRs.

## macos (most urgent)
1. **Main's macOS CI is red** since `1ddd658a`: `crates/agora/tests/secret_cli.rs` test `a_secret_is_in_no_output_of_the_node_the_cli_the_terminal_or_a_file` fails at line 198 with `no node` (the node started by `agora node run --project` never creates `node.sock` within 60 s). Last green: `addb6edb`. Four commits are in between (`530a6c68`, `0c56ae41`, `02bf50e1`, `282426df`) plus the test change `1ddd658a`. On a real Mac: build main, run that single test, and bisect the four commits; tell me which one and why (the node's stderr from the test is the thing to read). Linux and Windows pass it. Short AGORA_HOME paths matter on macOS (socket path length).
2. When the Mac is online again: update to the latest release (`agora update --version <latest> --release-key docs/security/release-key.pub`), `agora doctor`, and tell me whether the Local Network permission prompt appears again (the maintainer must re-grant it if it does).
3. After #761 merges (I will post here): run the holder tests on the Mac (`cargo test -p agora --test hold --test hold_node`); this code has only ever been compiled on macOS in CI, never used by hand.

## windows
1. After the next release: open the new web window in the Windows browser (rail on the right, grouping buttons on the left, chat screen of a structured agent) and tell me anything that looks wrong at your window size and scaling (125/150%).
2. Still open from before: a person-at-keyboard check of Ctrl+click on a link in the TUI on a real terminal; interface-address listing and firewall behaviour with `agora pair open`.
3. After #761 merges: check that on Windows nothing changed (there is no holder there yet; agents must still start and stop exactly as before, `agora down` and `agora down --agents` must not break).

Please answer with a note under `log/` with your platform in the file name. Keep it short and factual: commit, command, result. No real credentials, tokens, names or paths.
