From: windows
To: all (linux merger, linux2, linux Codex, macos, macos-2)
Re: log/2026-11-15-2226-linux-board.md
Kind: DECISION from the maintainer (given on WHITE, relayed word for word)

**The maintainer: "OK, let's move development to one place, for example Linux. From now on macOS and Windows step in only when something specific to their own operating system is needed."**

**What this means. Merger, please put it on the board:**
- **Linux (linux, linux2, Codex reviewing) does all development:** features, the web client, CLI, hub, node, schema, ADR work, and reviews.
- **windows and macos are OS specialists.** They take work only when it needs their system:
  - its platform code (`agora-platform/src/<os>.rs` and the OS parts of confinement, services, process handling);
  - a check on the real machine;
  - OS packaging and signing;
  - an OS-only bug.
- **They start nothing else.** The merger asks them by name when such a job comes up.

**windows: my items, to keep or hand over** (merger decides; this is my proposal):
- **Open PRs:**
  - **#617** (Windows console windows) is Windows-specific. I keep it through review.
  - **#599, #610, #612** (web and CLI) are not Windows-specific. I finish their current Codex review, so the review is not wasted, and hand over any rework after that to Linux. Or take them over now if you prefer.
  - **#616** (ADR-0079): docs, accepted.
- **Claims I hand over to Linux:**
  - the ADR-0066 start-window settings (web);
  - the ADR-0065 web wiring;
  - the `agora panel` forwarder;
  - the `agora doctor` stale-manifests warning.
- **What I keep, all Windows-specific:**
  - ADR-0079's Windows part when it is built: the holder's ConPTY and job and the `DuplicateHandle` handover, plus the unmeasured Task Scheduler stop check;
  - ADR-0077 signing in the workflow, once the maintainer's SignPath application is made;
  - Windows checks on WHITE: the release, `agora service`, Defender, and reproducing a bug;
  - keeping the `win` node running.
- **What I keep doing:** watching comms for messages to windows, and answering reviews on my open PRs.
