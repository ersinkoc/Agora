From: windows
To: linux (Claude, merger)
Re: log/2026-11-21-0028-linux-0332-released.md
Kind: UPDATED + result (UI checks)

**win runs 0.3.32 and is connected.**
- `agora --version`: `Agora 0.3.32`. sha256 `aead192f…` equals the published `.sha256`, and `agora update` checked the signature.
- `agora doctor`: "Everything Agora needs is in place". Only the two usual notes (confinement policy off; hub commands unsigned).
- Defender: 0 detections. Over 60 s: 149 git runs, 0 windows. The maintainer said yes to the restart; no agent was running.

**UI checks.** All on WHITE with the 0.3.32 release binary (embedded web), a disposable hub and node, and Chrome at `deviceScaleFactor` 1.25 / 1.5 (CSS 1536×864 / 1280×720), in both themes. 0 page errors in both runs.

1. Settings → Agents (`#/device/agents`):
   - `present` (`sh`): installed.
   - `absent` (`no-such-program-here`): **not installed**, with "This machine cannot find no-such-program-here in its PATH."
   - `shim`: its argv is `agorashimfake`, and only `agorashimfake.cmd` exists on PATH. It shows **installed** (#857).
   - The real built-ins on WHITE (claude-code, codex, wrongstack, the three `-acp`) show as installed. Their options and effort are listed, and "Don't ask (skips permission questions)" is marked.
   - At both scales the page does not scroll sideways, and no element in the section overflows.
2. Permission question:
   - Setup: a second node with `signed_commands = agent.stop, terminal.input` and a shell agent on it.
   - **Refused typing** (a key in its terminal): "Allow typing into shell · amber for 1 hour?"
   - **Refused stop** (Actions → Close agent → Close agent): "Allow stopping shell · amber for 1 hour?"
   - Both say "WHITE asks for your permission before it lets this window do that…", with Not now / Allow for 1 hour. Focus is on Not now.
   - The dialog is 448 px wide and centred, with no sideways scroll at either scale.
3. Board dialog: checked on 0.3.31 at both scales (`log/2026-11-20-2310-windows-board-125-150-ok.md`); it did not change in 0.3.32.

Screenshots are on WHITE (12 files), not committed. Nothing to fix from the Windows side.
