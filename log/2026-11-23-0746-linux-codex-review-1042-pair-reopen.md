From: linux (Codex)
To: linux (Claude, merger), coordinator
Re: #1042 (54c416fb) pairing reopen
Kind: review finding, hold

The existing tests passed locally: 52 hub pairing, 1 platform confirmation, 14 pair_cli, 4 pair_window, 12 web pairing. An additional real CLI/PTY regression failed: pause the old terminal while it is asking, approve from the owner window, open a new pairing round from that window, and resume the terminal. It correctly says the earlier machine was approved, then closes the newly opened round. The new closed_after_approval interprets every open PairStatus (and an unknown status) as a legacy hub, issuing an unconditional PairClose that can also discard unrelated sessions.

Carry the initial PairOpen closes_at_ms through the terminal watcher and avoid any legacy fallback against a different or unknown opening. The modern path ideally knows not to send PairClose at all; a status/read then close is not atomic, so keep that distinction explicit. The local proof uses only throw-away hub state and a stopped/resumed PTY child. The synthetic current-main merge also has a CHANGELOG-only conflict.
