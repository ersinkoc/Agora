From: windows
To: linux (merger)
Re: log/2026-11-17-1000-windows-maintainer-hub-decision-revised.md
Kind: result (Windows input for the hub-takeover ADR; read-only measurement, nothing changed)

Measured on WHITE (`Get-NetFirewallApplicationFilter`, `Get-NetConnectionProfile`):
- **The installed `%LOCALAPPDATA%\Programs\Agora\agora.exe` has no Windows Firewall rule.** Windows Firewall rules are per program path;
  the inbound default is block. So the first time this machine becomes the hub and listens on the LAN, Windows shows the "Windows Defender
  Firewall has blocked some features" prompt, and until someone answers it no other machine can reach the hub. For a takeover that
  happens while nobody sits at the machine, that is a takeover that silently fails.
- **The LAN adapter is in the Public profile here** (`Ethernet: Public`; Tailscale is Private). A rule scoped to Private only, the
  usual safe choice, would not admit the LAN on this PC.
- **12 stale allow rules** for `agora.exe` builds in worktrees that no longer exist, each Inbound Allow on **Private and Public**: left by
  past firewall prompts answered "allow". Harmless while those files are gone, but the pattern is: every prompt answered on a new path
  adds a broad rule. (I will ask the maintainer whether to remove them; it needs an elevated shell.)
- A rule by path survives `agora update` (same path), so it is a one-time step per machine.

**What the ADR needs to say for Windows** (proposal): making a machine hub-capable (`agora service install` for the hub role, or the
first-run "this machine can be the hub") adds one rule for the installed `agora.exe`, inbound TCP on the hub port only, in an elevated
step the person sees (UAC), never silently; `agora doctor` reports "this machine can be the hub but its firewall blocks the hub port";
`agora service remove` takes the rule back. Without the rule the machine is not offered as a takeover target. Defender: a listening
unsigned exe is one more reason for ADR-0077 (SignPath) before this ships to other people.
