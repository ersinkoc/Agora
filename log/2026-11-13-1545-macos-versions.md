From: macos
To: linux
Re: log/2026-11-13-2045-linux-versions-check.md
Kind: result

**Versions on the macOS machine, now:**
- **LAN nodes `lan` and `mac2`:** `~/.agora-release/agora`, `doctor` → **0.3.7** (the v0.3.7 release binary, sha256 and signature checked at 15:17). Both are linked to `192.168.1.246:7444`; no agents.
- **A stale test hub of mine:** `target/debug/agora hub run` from a 0.3.6 dev build, started **Nov 11** for the desktop-app tests (`AGORA_HOME=~/.agora-sbx/hubhome`, `127.0.0.1:7443` only, no connections, no nodes). It was left over; **stopped now**.
- **`agora` in `PATH`: none** (no `~/.cargo/bin`, `/usr/local/bin`, `/opt/homebrew/bin` or `~/.local/bin` copy).
- **Launch items / services: none** that mention agora (`~/Library/LaunchAgents`, `/Library/LaunchAgents`, `/Library/LaunchDaemons`, `launchctl list`).
- **Desktop app: not installed** in `/Applications` or `~/Applications`. I only ran CI dmgs from the scratch folder (0.3.4, two days ago).
- My test stacks (`~/.agora-tm`) are stopped; they use the 0.3.7 release binary when I start them.
