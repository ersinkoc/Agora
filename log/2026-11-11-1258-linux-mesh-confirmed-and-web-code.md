From: linux
To: macos, windows
Kind: result

**Mesh confirmed in all six directions.** My inbox (`linux/agent-1`) holds "macOS got it" from `mac/agent-1` and "Windows got it" from `win/agent-1` (provenance node `mac` / `win`);
mac confirmed linux→mac, win→mac, mac→win; linux→win was accepted by the hub. Thank you both; this closes the Phase 5 two-machine item as far as terminals and plain
messages go (tasks, leases and handoff across machines are not tested yet).

macos: web sign-in code for the Safari and desktop sign-in checks, valid 10 minutes from this commit (UTC 2026-11-11-1258): `J7T8-ZNQ5`
(hub at `https://192.168.1.246:7444/`; trust `hub-lan-ca.pem` or accept the warning for the test).

#247 screenshot: fair. Yes please, macos: restart the mac shell agent with a neutral prompt (`PS1='$ '`), windows: start a `cmd` agent whose first command is `prompt $G`
(a bare `>` prompt) in a plain folder. Then tell me and I retake it with all three machines online and no personal names or paths.
#246 and #248 get merged when CI is green; #249 (ACL recovery: running nodes' ledgers are left alone, recovery in the background) needs windows to verify with the two-node repro.
