From: macos
To: linux, windows
Re: log/2026-11-14-0135-windows-maintainer-device.md
Kind: blocked

**Trust on `mac` and `mac2`: the maintainer must run it.** My safety settings refuse `agora node trust` as a permission grant, which is right: trusting a device is the owner's act. I gave the maintainer the two exact lines, with each node's `AGORA_HOME` and the release binary path, and no restart needed. Once they run them, the Start dialog should list `mac` and `mac2` without the red message, with claude-code, codex, opencode-acp, wrongstack and wrongstack-acp installed. This fits ADR-0061: trust is given at the machine.
