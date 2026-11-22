# Real-machine follow-ups, 2026-11-22

Three results reported through the sessions' coordination log on 2026-11-22, written down here so `docs/capabilities.md` and the
checklist can cite them. Each was run by the session on that machine; no real agent account or token was used.

## Windows (WHITE): `agora update --check` after #952

- Build: #952 at `de8611dc`, release build, run from PowerShell with the inbox `C:\WINDOWS\system32\curl.exe` first on `PATH`
  (Git Bash puts Git's own curl first, so a first run there did not count).
- `cargo test --locked -p agora --test update update_check_follows_the_latest_redirect_and_reports_a_missing_release`: 1 passed.
- `agora.exe update --check` against the real release address: "Agora 0.3.33 is the newest release." (exit 0). With a wrong
  `--release-url`: "Could not check for updates. The newest release could not be found." (exit 1).
- So the 2026-11-21 failure (#939, [Windows 2026-11-21](2026-11-21-real-machine-windows.md)) is fixed on that machine.

## All three machines updated to 0.3.34

- **win (WHITE):** `agora update --version 0.3.34 --release-key docs/security/release-key.pub` said "Updated to Agora 0.3.34"; the
  installed `agora.exe`'s sha256 equalled the published `.sha256`; 0.3.33 kept as `agora-0.3.33.exe`. After `agora down` and the
  logon task: `agora --version` 0.3.34, `agora status` connected to the hub as `win`, `agora doctor` all ok, Defender silent.
- **mac (Apple M4):** the same update command; the installed sha256 equalled `agora-aarch64-apple-darwin.sha256`. After the launchd
  restart `agora status` named the Local Network block (os error 65), as after every unsigned update (ADR-0086); once allowed and
  restarted: 0.3.34, connected as `mac`, `agora doctor` with no failing line.
- **blackview (hub machine):** hub and node updated with the same release key and restarted; hub 0.3.34 and node 0.3.34, connected
  as `blackview`. All three machines were online on the hub afterwards.

## macOS (Apple M4): the mesh after a node restart, and a message to a stopped machine

Release build of `main` at `c32c19d0` (contains #976), macOS 27.2.1, a scratch install: a hub on 127.0.0.1 and two nodes, `mac` and
`rm-two`. Each mesh call went over its own new `agora mesh stdio`.

- **A kept agent reads new messages after the node restarts (#965):** before the restart, `inbox` of the kept `mac/agent-1`
  returned 3 items. `agora down` ("2 agents keep running"), then `agora`: `agents_list` identical before and after; `inbox` then
  returned the 4 messages posted after the restart, and `inbox` with `since` set to the old cursor returned the same 4. This is
  evidence for the mesh bridge reconnecting (ADR-0075 step 1); the later steps of ADR-0075 are not built.
- **A message to a stopped machine (RM-27, #915):** `kill -STOP` of `rm-two`'s node; after 30 s the hub showed it offline. A
  `send_message` from `mac/agent-3` to `rm-two/agent-1` returned a `message_id`; about 60 s later the sender's `inbox` held one
  `delivery_failed` from `rm-two/agent-1` with body `node.offline` and `reply_to` that `message_id`. After `kill -CONT` the recipient's
  `inbox` was empty: the failed post was not delivered later. #915 was closed on this result.
