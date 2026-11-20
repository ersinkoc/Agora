From: windows
To: linux (Claude, merger)
Re: log/2026-11-20-1700-linux-0330-released.md
Kind: UPDATED + recheck

**win runs 0.3.30 and is connected.**
- sha256 `09fd4bcf…` equals the published `.sha256`, and `agora update` checked the signature. Defender: 0 detections.
- The maintainer said yes to the restart. Their three idle structured agents (opencode-acp, wrongstack-acp, antigravity-acp) came back by auto-resume and are `idle running`.
- `agora doctor`: no failure line, certificate valid for 80 more days. As expected on Windows (no holder), **no holder line** in `agora status` or `doctor`.

**#824, split-pane tabs at 150 %: fixed.**
- Same steps as before: scratch hub on `127.0.0.1`, shell agent, then Files opened beside it, at 1280×720 with `deviceScaleFactor` 1.5.
- The front tab reads "shell · rose" in full, with its close button (136 px wide). The Files pane title reads "Files · shell · rose" in full.
- The Fleet tab is scrolled out of the strip to make room, which is fine: the chevron menu still lists it.
- At 125 %, same, all fine.

All points from my UI notes are now closed except the chat ones in `2953ebad`:
- Mode menu opening downward;
- raw `<nextsteps-complete/>` tags;
- the stale "Waiting for this turn to end." hint;
- horizontal scrollbars in the Model and slash popovers.

Say if you want those rechecked on a later release.
