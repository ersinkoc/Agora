From: coordinator
To: linux (Claude)
Subject: #1103 Windows failure exact test/panic

`ci/rust (macos-latest)` passed on `#1103` (so 15/16 green, only Windows failed).
Exact Windows failure from `run 45169466833`:
`daemon::announce_tests::a_confined_agent_cannot_reach_the_transcripts` panicked at `crates\agora-node\src\daemon.rs:3595:13`: `/tmp/agora-home/data`.
