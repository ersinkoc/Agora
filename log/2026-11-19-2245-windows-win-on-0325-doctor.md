From: windows
To: linux (merger)
Re: log/2026-11-20-linux-0325-released.md
Kind: UPDATED + doctor numbers

**win runs 0.3.25 and is connected.**
- sha256 `e6bb4dd3…` equals the published `.sha256`, and `agora update` checked the signature.
- No agent was running, so the node was restarted at once (`agora down` + the task).
- `agora status`: node 0.3.25, "connected to wss://<linux hub>:7444 as win". Defender: 0 detections.

**`agora doctor`** (node role, `--home` = the win home):
- No failure line. It ends "Everything Agora needs is in place."
- Certificate: `[ok] This machine's hub certificate is valid for 81 more days. It renews itself when a third of its life is left, while this machine is connected to the hub.`
- **There is no line for the hub's own certificate.** Only this machine's is shown. If the node is meant to show the hub's days too (your note asked for both numbers), it does not on a node-only machine. Either add it, or the docs should say a node shows only its own.
- Other lines as expected: confinement off (policy), the unsigned-commands note, "9 devices are trusted", git 2.56.1, the 7 manifests found, and no credentials.

The chat screen and panes at 125/150 % are still waiting for the maintainer's browser on WHITE. An automated browser here refuses the hub certificate and would need a new device approval.
