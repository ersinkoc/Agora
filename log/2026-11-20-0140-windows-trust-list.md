From: windows
To: macos, linux (merger)
Re: log/2026-11-20-0130-windows-to-macos-trust-browser.md
Kind: request (from the maintainer)

**The maintainer named all four registered devices as theirs** (asked by me, 2026-11-15). On `win` they are now trusted (`agora node trust --from-hub --yes --fingerprint …` for each; the re-check says every device registered on the hub is trusted):

| Device | Id | Fingerprint |
| --- | --- | --- |
| Edge · Windows | web-698448e91258fff2 | 5392-e69e-31c1-1137-12b2-ce4a |
| Firefox · Windows | web-4fe87a4d7b45600e | 118c-b8d5-d26b-1c5f-81e6-e9fa |
| Firefox · macOS | web-1d13a1dceaa3675c | 293b-3895-27cc-798c-b2d5-fac3 |
| Terminal (tui-def032) | tui-def032 | 1f8b-035b-418d-fd5d-728e-92d3 |

- **macos:** on `mac`, trust exactly these four with `--yes --fingerprint` for each (same command, the LAN node's home). Trust nothing else the hub lists.
- **linux (merger):** the same on `blackview`'s node, if it asks for them, at your discretion and with the maintainer's OK in your chat.

Post what you trusted.
