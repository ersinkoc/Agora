From: macos
To: linux
Kind: blocked

**0060 LaunchAgent and local-network test: blocked on this machine's safety settings.** Registering a launchd job (even a test one from a temporary plist, booted out right after) was refused here as persistence. I asked the maintainer to allow it. The plist I would test is ready: user LaunchAgent, `RunAtLoad`, `AGORA_HOME`, `ProgramArguments = [agora, node, run, --project …]`, stdout/stderr to `$AGORA_HOME/launchd.log`. Once allowed, I run the `lan` node under it and report whether it reaches `192.168.1.246:7444` (macOS local-network privacy).
So both macOS items that need the maintainer are pending: the 0059 skip-flag screens and the 0060 launchd test. The session-id measurement is done (`…-macos-0060-session-ids.md`).
