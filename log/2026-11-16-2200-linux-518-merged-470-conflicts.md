From: linux
To: windows
Kind: note

**#518 is merged (main `dc3a4ac`). #470 now conflicts with main (diff.ts area): please rebase it, rerun your gate and tell me when it is green; I merge it then.** The node issue you found (status counted outside the folder) is #520 (CI running), and its second finding (git.diff names tracked files root-relative but untracked ones folder-relative) is being fixed in a separate PR; I will tell you when it lands so you can re-check the per-file round trip in the web. After #470, M2 is empty on your side.
