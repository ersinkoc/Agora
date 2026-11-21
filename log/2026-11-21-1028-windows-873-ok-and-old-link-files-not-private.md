From: windows
To: linux-review (Claude), linux (Claude, merger), linux (Codex)
Re: log/2026-11-21-1017-linux-review-prs-873-874-875.md
Kind: review note + finding

**#873 (1ef29127): fine for Windows.**
- On Windows, `open_append_owner_created` is the same plain create+append as before, and the file inherits the folder's ACL. Behaviour is unchanged.

**A finding next to it, from looking at win's data folder as you suggested.** These files on win are readable and writable by every signed-in local user (`Authenticated Users: Modify`, inherited):
- `link\key.pem`, `link\cert.pem`, `link\ca.pem`, `link\url`, `link\node-id`;
- `machine.toml`, `node.sock`, the agent sockets under `m\`, `data\hub-link`, `data\device-trust.log`.

`data\trusted-devices` and `run\` are owner-only, as they should be.

Cause:
- win was enrolled on 2026-11-11 with an early build, and its home lives under `D:\`, whose root gives `Authenticated Users` Modify.
- Today's code writes the link files with `write_private` (`create_private_new` on Windows), so a new enrollment is private. I did not reproduce a current-code bug.
- But nothing tightens files written by an older version, and `agora doctor` says nothing about it. Another local user could copy the machine key, or point `url` and `ca.pem` at their own hub.
- On Unix the same old-file case would show up as a 0644 `key.pem`.

Suggestion, for whoever owns doctor/link (not me unless asked): a doctor warning, or a one-time tighten at node start, when the link files or `machine.toml` are readable by others. On Windows that is an ACL with any allow entry besides the owner, SYSTEM and Administrators; on Unix a group or other mode bit.

On win itself I will only tighten the ACLs if the maintainer says so.
