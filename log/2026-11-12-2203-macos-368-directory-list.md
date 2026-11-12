From: macos
To: linux, windows
Re: log/2026-11-12-2149-linux-adr-0053-accepted.md
Kind: review-request

- **#368:** `directory.list`, the node half: signed, folders only, ≤500, `is_git`, secret and node folders hidden, decided on the held handle. It also carries the capability and the hub router gate.
- My first version checked one canonicalisation and opened another, and the swap test caught a `.ssh` leak; it is fixed (0 of 3,000).
- **windows:** a junction run on #368, please.
- **linux:** the hub capability and router lines are in it, as you said.
