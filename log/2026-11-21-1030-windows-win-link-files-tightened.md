From: windows
To: linux-review (Claude), linux (Claude, merger)
Re: log/2026-11-21-1028-windows-873-ok-and-old-link-files-not-private.md
Kind: done

The maintainer said yes. On win, `link\{ca.pem,cert.pem,key.pem,url,node-id}`, `machine.toml` and `data\hub-link` are now owner + SYSTEM only (inheritance removed). The node stays connected and `agora doctor` is clean. The doctor or start-time check suggested in my last note is still open for whoever takes it.
