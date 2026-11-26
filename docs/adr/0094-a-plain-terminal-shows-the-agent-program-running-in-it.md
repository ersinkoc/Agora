# 0094. A plain terminal shows the agent program running in it
Status: Accepted
Date: 2026-11-20
Accepted: 2026-11-20 (the maintainer's explicit message in the merger chat: "ADR-0094 ve ADR-0085 Ek A'yı kabul ediyorum"). The open questions' proposed answers stand.

Accepted by the maintainer (see the Accepted line above): this may now be built.

Built (2026-11-26, on `main`): build step 1 and the node side of step 2 without the schema field. The names live in the existing
`[process] names` (documented as foreground process names and already set by the shipped manifests) rather than a new `[match]` table, so
no engine version was needed; they are now validated. The node looks every 2 s at a watched or not-idle `shell` agent's foreground program
and reads it with the matched kind's screen rules, so state and needs-you work for a hand-started agent; the mark ends with the program.
Step 2's schema part (approved by the maintainer on 2026-11-26: "onay veriyorum"): the event `AgentRunsInside` (`Event.body` 90) and
`AgentSnapshot.runs_inside` (46); the node records the mark's changes and says it again after every `AgentStarted` it restates; the hub keeps
it for a running agent and clears it with the process (`docs/specs/protocol.md`, "What runs inside a plain terminal").
Step 4 (2026-11-27): the window's "Claude Code in Shell" chip on the Shell's tile, pane header and phone row, named by the machine's own
name for the kind when the window has it (else the kind id's words), and `[Claude Code in Shell]` on the terminal client's line; the Shell
keeps its own name and avatar (`docs/ux/main-window.md`).
The registry's names (the second open question, 2026-11-27): rather than the converter writing them (its golden outputs are never
edited), the node takes a kind's ACP program as its name when the kind lists none and that program is a plain file name of its own
(`docs/specs/agent-manifest.md`).
Not built: the Windows lookup (step 3, measured on Windows first).

Builds on ADR-0004 (agents are data), ADR-0079 (the terminal holder), ADR-0091 (plan limits) and the invariant "detection never reads the
user-visible viewport". Touches the threat model (a program name is untrusted data).

Origin: the maintainer, 2026-11-20: "I open a plain terminal and then run claude, codex, wrongstack inside it myself: will we notice them?"

## Amendment A: the Windows lookup (2026-11-27)

Approved by the maintainer on 2026-11-27 ("Evet, değiştir"), after step 3 was measured on a Windows 11 machine (about 800 processes) with
Agora's own pseudoconsole and job setup.

- **The rule.** The program in front is the newest direct child of the console's shell (a child created after its parent, by parent
  pid), followed down while that child is a shell (`cmd`, `pwsh`, `powershell`, `sh`) or a `cmd.exe` running a `.cmd`. "The newest
  descendant" is not stable: Claude Code starts `conhost.exe`, `reg.exe` and `git.exe` helpers and Codex starts `conhost.exe`, so at
  10 ms polls the newest descendant changed 6 to 10 times in the first 1.3 s; the newest direct child never did.
- **What it recognises.** A native install (`claude.exe`) and a bun install (`codex.exe` shim, then `node.exe`, then `codex.exe`) match by
  image name. An agent installed as an npm `.cmd`/`.ps1` shim shows only as `node.exe` and is not recognised (arguments are never read).
  A program started in the background (`start /b`, `Start-Process -NoNewWindow`) is a direct child too and stays marked while the prompt
  is in front, since Windows has no foreground group; the window's help says both.
- **The cost.** The terminal's job object misses programs started from Store-packaged PowerShell (the default `pwsh` install), so the
  lookup reads one Toolhelp snapshot of the machine (p50 14 to 30 ms, p95 27 to 44 ms, max 53 ms), shared by every terminal and taken
  at most once per 2 s tick, only while some Shell is watched or not idle (about 1% of a core). An ended program leaves the tree within
  35 ms, so the mark ends at the next tick.
- Built in `agora-platform/src/winproc.rs` (with the other Windows process lookups) behind the existing foreground-program call; nothing
  else changes.

## Context
- A plain terminal is the `shell` kind. The node knows the shell it started and nothing about the programs the person runs in it. A
  Claude Code, Codex or WrongStack started by hand shows as an idle shell: no kind, no state (working, asks you, idle), no needs-you card,
  no chip, and none of the manifest's screen rules apply, because those rules belong to a kind the agent was started as.
- Starting an agent from the window already gives all of that. The gap is only the hand-started program.
- The node holds the pseudo-terminal and the process tree below the shell, so it can see which program is in the foreground without
  reading the screen. This is the signal terminals themselves use (the foreground process group).

## Decision
1. **The node looks at the terminal's foreground program, not at the screen.** On Unix, the foreground process group of the agent's PTY
   (`tcgetpgrp` on the master) and the executable of its leader. On Windows, which has no foreground group on a pseudoconsole, the
   newest direct child of the shell that owns the console, followed down while that child is itself a shell (Amendment A; this
   sentence said "the newest descendant" before it). The lookup is OS-specific and lives only in `agora-platform`. It runs at a low
   fixed rate (about every 2 s, only while someone watches the agent or the agent is not idle) and is never in the terminal byte path.
2. **Names map to kinds through data.** A manifest may carry `[match] executables = ["claude", "claude.exe"]`. The node compares the
   executable's file name (never its arguments, never its path contents) with the installed manifests. The first match marks the shell
   agent as "running KIND", and that kind's screen rules (ADR-0004 L1) start reading the live bottom of the buffer, exactly as for an
   agent started as that kind. When the program ends and the shell is back, the mark ends.
3. **The shell stays a shell.** The agent's kind does not change, its id does not change, its ledger entry and resume rule do not
   change. A new, additive field (a name, not a command) says which kind runs inside; windows show "Claude Code in Shell" and the state.
   Nothing the agent could print can set it: only the node's process lookup does.
4. **Nothing is installed or run.** Looking at a process is read-only. The node never starts the program, never sends it input, and a
   mark grants no permission: skip-permissions classification (ADR-0090) applies only to agents started as that kind.
5. **Unknown programs are ignored.** A program no manifest names is not marked; an executable that is renamed is not recognised, and
   that is stated in the window help rather than guessed from screen text.
6. **Quota and usage** (ADR-0091) keep reading the tool's own files per machine, so a hand-started Codex already counts there;
   the mark lets the window show the chip on that terminal as well.

## Alternatives
- **Read the screen for a banner or prompt.** Rejected: the invariant forbids it, and it can be spoofed by anything printed.
- **Wrap the shell so a launcher reports what it starts** (shell integration). More precise, but it edits the person's shell
  configuration, which Agora never does.
- **Ask the person to start agents from the window.** Already possible; this ADR is for the person who does not.

## Build order
1. `agora-platform`: foreground-program lookup (Unix) with tests on a fake process tree; the executable name only.
2. Manifest `[match]` field (append-only) and the node's mark; a snapshot field for "runs inside" (schema, append-only).
3. Windows lookup (descendant of the console's shell), measured on a real Windows machine before it is built.
4. The window: the "X in Shell" label, state and chip; the TUI line.

## Open questions
- Cost of the 2 s lookup with many terminals. Proposed answer: only terminals that are visible or not idle; measure before enabling it
  everywhere (ADR-0022).
- Is a node-side `[match]` enough, or should it also read the registry's command names (ADR-0092)? Proposed answer: the converter
  fills `[match]` from the registry entry's command when it is a plain binary name.
- Should a mark survive the node restart? Proposed answer: yes through the holder, because the lookup runs again on reattach.

## Consequences
Schema: one optional field in the agent snapshot, and a manifest key; both additive. Docs: `docs/specs/agent-manifest.md`, the threat model
row "foreground program name". Tests: a fake agent started under a shell, state changes and the mark ends when it exits; a renamed
binary is not marked; a program name containing control characters is refused.
