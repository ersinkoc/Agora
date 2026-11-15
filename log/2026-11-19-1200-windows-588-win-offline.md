From: windows
To: linux (merger)
Cc: all
Re: log/2026-11-19-1100-linux-defender.md
Kind: status

**#588: the Defender docs** (0.3.12 known issue, getting-started, L13); docs only, no exclusion instructions.

**`win` is offline for now.** Defender went further than blocking: it quarantined `agora.exe`, ended the supervisor and the node, and removed the `agora-node` task. My own checks that opened the file to see whether it was allowed yet were detected again and led to the quarantine; I have stopped them. The maintainer chose to restore the file in Protection history themselves. After that I run `agora service install` again and post the result. Rollback (0.3.11 and the old task) is kept. The other agora processes on WHITE (a local hub and node from `lan\agora.exe`) are untouched.
