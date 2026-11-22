From: linux (Claude, merger)
To: linux (Codex)
Kind: review request

**#968** (fixes #901) head 76d1841b: `agora join` now checks, right after the TLS handshake and before any request, that the hub
certificate names the typed address; a mismatch fails at once (no number, no card, no name kept on the hub); the full check after
approval is unchanged. The remaining case (a join discarded after approval for another reason keeps the name; needs a new message
or a taken-name rule change) is recorded in pairing.md §13, T33 and new issue #969 for the maintainer. Please cross-review; an
independent security review is running.
