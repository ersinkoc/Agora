# 0072. A shared terminal takes the size of the smallest window looking at it
Status: Proposed
Date: 2026-11-16
Origin: the maintainer, 2026-11-15: they look at an agent's terminal in the web window, then look at the same agent from another machine's bigger window; the terminal changes size and no longer fits the first window.

## Context
A terminal has one size, because it is one PTY and one screen. Every window that watches it sends `terminal.resize` when its viewport changes, and the node applies each one at once and tells every watcher (`Output::Resize`). The last window to resize wins: a bigger window makes the terminal bigger than the smaller window's viewport, so the smaller window shows a terminal that does not fit. ADR-0029 says the hub relays commands and "never fabricates or edits a command's content"; so the hub currently cannot decide a size, and no one else can know every window's size.

## Decision
1. **The size of a shared terminal is the smallest columns and the smallest rows over all windows that are looking at it** (tmux's `smallest` policy), so nobody is clipped; a bigger window shows the smaller terminal at its top left on the surface colour. When the smallest window leaves, the terminal grows back to the next smallest.
2. **The hub decides it.** For each terminal it keeps the last size each subscribed window reported, forwards the minimum to the node only when the minimum changes, and forwards a new minimum when a window unsubscribes, disconnects or is refused. A window that is not subscribed to a terminal has no say while any subscribed window has a size; with nobody watching through the hub a resize goes through unchanged, so scripts keep working. Sizes are clamped to the node's own limits; read-only viewers cannot resize at all (unchanged).
3. **This amends ADR-0029 in exactly one place:** `terminal.resize` is the one command whose size the hub chooses, and the hub may send one on its own when the set of watchers changes. It changes a size only, never what policy allows, and it needs the same grant a resize needs today. Everything else in ADR-0029 stands.
4. **No schema change.** Clients still report the size that fits their viewport and show the size the node reports; they must not report again because of a size frame from the node (no loops).
5. **Local clients on the node's control socket** (the terminal client, the CLI) talk to the node directly, not through the hub; for them the last resize still wins. The hub reads the terminal's own resize frames so it never fights a local client.

## Consequences
- A tiny viewer (a phone, more so with its keyboard open) shrinks everyone's terminal while it watches. That is inherent to "smallest"; it ends when the viewer leaves.
- A frozen client keeps its size until its connection ends (the window's ping bounds it).
- Two panes of the same terminal in one window share one size (separating them would need a stream id on the resize command: a schema change, not proposed here).
- The hot path (terminal frame relay) is unchanged: 94–98 ns per frame on main, 97–99 ns with the change.
## Alternatives considered
- **Last window wins (today):** the maintainer's bug.
- **The window you are typing in wins:** clips the others while you type, and it needs input tracking in the hub.
- **Each window gets its own size (a per-window virtual terminal):** a different product: the terminal is one screen.
## Open questions
- Should a window be able to say "I am only looking, do not count my size" (a read-only mirror)? Proposed: not now.
