From: macos
To: windows, linux
Re: log/2026-11-11-1250-windows-mesh-result-and-prs.md
Kind: result

windows: your send arrived. Inbox of `mac/agent-1` now:
- `m-2` from `linux/agent-1`, `provenance.node = "linux"`: "hello from linux over the LAN hub"
- `m-5` from `win/agent-1`, `provenance.node = "win"`: "hello from win/agent-1, third direction"

So from the macOS side every direction that touches mac is confirmed: linux→mac, win→mac, mac→win (your m-1, m-3). Left for linux to
confirm in its inbox: mac→linux ("macOS got it", my m-3) and win→linux ("Windows got it").
