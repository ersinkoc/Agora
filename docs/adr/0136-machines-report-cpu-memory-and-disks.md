# 0136. Machines report their CPU, memory and disks
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (maintainer, with the schema step and the proposed thresholds).

## Context

The maintainer asked (2026-11-27) to see every machine's CPU, memory and disks together, and that disks never fill. `NodeResources`
carries only agents running, terminals open and the running agents' memory, sent when those change: nothing about the machine itself. ADR-0064 decision 5 already decided that the node reports free space and its workspaces' size and
that the window raises "Needs you" when a disk runs low; that part waits on a schema change (ADR-0064 "Not built"). A disk that fills
stops builds, git, the node's own event log and the agents with it, so it must be seen before it happens, on every machine at once.

## Decision

1. **The node measures its machine.** Every 30 s, off the hot paths, the node reads: the number of logical CPUs and the share of CPU
   time the whole machine was busy since the last reading; total and available memory (available as the OS counts what can be given
   without swapping); and each local file system that holds data (not `tmpfs`, `proc`, overlay or other pseudo file systems, at most 8,
   deduplicated by device): where it is mounted, its total size and the space this user may still use. It marks the file systems that
   hold its own data folder and its workspaces. The size of its workspaces is the figure ADR-0064's clean-up already keeps. Reading is
   platform code (`agora_platform::machine_stats`, `disk_usage`, one per `<os>.rs`); a platform that cannot read a figure leaves it out.
2. **It says so in `NodeResources`, only when it changes.** Every report the node already makes carries the latest figures; it also
   reports at start and when a figure moves past a step (CPU by 5 points, memory by 2 % of total, a disk by 1 % of its size or 1 GB, a file system appearing or going), and at most
   once a minute, so a busy machine adds at most 1440 small events a day and a quiet one almost none. Terminal bytes are untouched.
3. **Schema, append-only.** `NodeResources` gains `cpu_count` (5), `cpu_busy_permille` (6, 0–1000 of all CPUs), `memory_total_bytes`
   (7), `memory_available_bytes` (8), `repeated DiskUsage disks` (9) and `workspaces_bytes` (10); `DiskUsage` is `mount` (the mount
   point or drive as the OS names it), `total_bytes`, `free_bytes`, `holds_data`, `holds_workspaces`. `NodeSnapshot` carries the latest
   of each (fields 12–17) and when they were recorded (`resources_at_ms`, 18) so a window opened later sees them. A field left at zero means "not reported", never "zero". The existing
   `memory_used_bytes` keeps its meaning (the running agents' resident memory).
4. **The window shows them together.** The machines list and each machine's page show CPU (busy share and count), memory (used of
   total) and every disk as a bar with free space, the workspaces' size beside the disk that holds them, with design tokens only and in
   the UI vocabulary. A disk is **low** when its free space is under 10 % of its size or under 5 GB, **critical** under 3 GB (the
   node's existing `workspaces.min_free_gb` default, below which it already refuses new worktrees). A low or critical disk on any
   online machine raises "Needs you" with the machine, the disk, what is free, and the way to clean up (ADR-0064's clean-up; its window
   buttons follow as their own step). An offline machine shows its last figures with when they were measured.
5. **The node keeps doing what it can on its own.** ADR-0064's automatic clean-up of what the node created and its refusal of new
   worktrees below the minimum stay as they are; this decision adds sight, not new deletion.

## Consequences

- Every machine's CPU, memory and disks are on one screen; a filling disk is a "Needs you" before it stops anything.
- A small, bounded stream of events per machine; the hub keeps only the latest per node in its projection.
- Each platform needs its reader: Linux first (`/proc/stat`, `/proc/meminfo`, `/proc/self/mountinfo`, `statvfs`); macOS and Windows
  report nothing until their sessions add theirs, and the window says "not reported" for them.

## Alternatives considered

- **Poll from the window through a command:** no history while the window is closed and nothing for "Needs you" on a phone; the event
  also reaches the hub's audit and an offline machine's last state.
- **Only the data disk:** simpler, but the maintainer asked for all disks, and builds often fill a second volume.

## Open questions (with proposed answers)

1. Thresholds: 10 % or 5 GB for low, 3 GB critical (proposed); ADR-0064 decision 5's "10 % or 20 GB" warns too early on small VPS disks.
2. Every 30 s reading and the change steps above (proposed).

## Status of the build

Built (2026-11-27): the schema (`NodeResources` 5–10, `DiskUsage`, `NodeSnapshot` 12–18 with `resources_at_ms`; fixture test
`crates/agora-protocol/tests/node_resources.rs`); the Linux reader (`agora_platform::machine_stats`, `disks`); the node's gate
(`crates/agora-node/src/machine.rs`, read every 30 s in the supervise loop, said on a step and at most once a minute, and with every
other report); the hub keeps the latest figures per node and puts them in the snapshot; the window's machine cards show CPU, memory
and every disk (low in amber with words, critical in red with words, an offline machine's last figures with when), My machines counts
the disks running low, and Home's "Needs you" leads with them. Left (follow-ups): the macOS and Windows readers (their sessions), the
phone's view of them, and ADR-0064's clean-up buttons in the window.
