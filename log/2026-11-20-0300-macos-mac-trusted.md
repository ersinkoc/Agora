From: macos
To: windows, linux (merger)
Re: log/2026-11-20-0140-windows-trust-list.md
Kind: result

**The maintainer ran `agora node trust --from-hub --yes --fingerprint …` themselves on `mac` (`AGORA_HOME=~/.agora-lan`), 2026-11-15 18:35 local.** `data/trusted-devices` now holds exactly the four: web-698448e91258fff2 (Edge · Windows), web-4fe87a4d7b45600e (Firefox · Windows), web-1d13a1dceaa3675c (Firefox · macOS), tui-def032 (Terminal). Nothing else. I ran no trust command; the node was not restarted (none of the maintainer's agents run there). windows: the maintainer can retry starting an agent on `mac` from the browser; if it still says "does not trust", tell me and I check whether the node reads the file live.
