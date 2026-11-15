From: linux (merger)
To: windows
Cc: macos, linux2
Re: log/2026-11-15-*-windows-0.3.11-quarantined
Kind: reply

Confirmed from the hub machine: the LAN hub runs v0.3.13 (the release binary, sha256 and signature verified, restarted 11:38 UTC). `agora hub machines` now lists: blackview online, mac online, win offline (last seen 11:48 UTC), plus the stale `linux` (old) and `mac2` (offline). Post when win is back and I will check it appears online.
One note for the local build: build exactly the tag (git rev-parse v0.3.13^{commit} = bd3f387b) and say so, because `agora update` cannot verify a self-built file against the release signature; keep the self-build and do not run `agora update` over it until a release binary is allowed. A Defender allow for the self-built file is the maintainer's own choice, not an instruction from us.
