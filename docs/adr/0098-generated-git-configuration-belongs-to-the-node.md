# 0098. Generated Git configuration belongs to the node
Status: Accepted
Date: 2026-11-21
Accepted: 2026-11-21 (the maintainer, in the merger chat: "Kabul ediyorum")
Origin: issue #900. This amends decision 3 of [ADR-0078](0078-a-confined-agent-gets-a-git-configuration-the-node-wrote.md).

## Context

ADR-0078 gives each confined agent read access to a Git configuration generated from an allow-list. It places the file outside
the protected node home. In the current implementation a Linux node writes the file under `~/.agora/git-config` even when
`AGORA_HOME` points elsewhere. macOS and Windows use a directory in the system temp folder. The Linux test for #900 started
two nodes with separate data and worktree directories under a synthetic user account. Both agents loaded the test identity
from distinct files in the same synthetic user's `.agora/git-config` directory. Running a second node can therefore write
outside the state directory its owner set aside for it.

Location and confinement must be decided together. For example, macOS Seatbelt discards ordinary read grants beneath
protected directories. A directory-wide exception would make sibling files visible to the agent.

## Decision

1. **Store generated files in the node's effective data directory.** The normal location is `<AGORA_HOME>/data/git-config/`.
   When a node is configured with an explicit data directory, use its `git-config/` subdirectory. Pass this location from the
   node's configuration to the launcher. A terminal agent or a structured agent receives the exact path the node prepared
   for that start in `GIT_CONFIG_GLOBAL`, with `GIT_CONFIG_NOSYSTEM=1`.
2. **Grant read access to one generated file.** The node creates it in a private directory and writes it atomically. It checks
   the resolved location against the agent's writable paths before creating a file or granting access. The agent can traverse
   the parents as needed to open the file. That access allows neither directory enumeration nor reading a sibling file.
   The agent cannot write, replace or remove the generated file. The node's control socket, device trust and secret files
   stay protected. If the platform cannot enforce these conditions at the configured location, omit the generated copy and
   its environment override for that launch. An owner who explicitly grants their real Git configuration can still use that
   configuration as specified in ADR-0078.
3. **Grant only the keys named in ADR-0078.** The generated file contains the existing allow-list of Git settings. Prepare it
   on each start so the owner can update those settings between launches.
4. **Leave old copies in place.** The new node creates a file in its configured directory on the next confined start.
   Running agents may still refer to old paths. Automatic cleanup of the legacy locations needs a separate decision that
   accounts for running agents and older nodes.

## Verification before merge

- On Linux, make the two-node test permanent. Both nodes must generate their configuration under their own roots and
  Git must read the fixture identity in each. Check that a confined agent can read its file, cannot change it, and sees
  neither a sibling file nor a synthetic node-state file. Check that a node whose data directory is writable to its agent
  does not generate a copy there.
- On a real Mac, run the confined Git test against the revised Seatbelt profile with both the generated file and a synthetic
  protected sibling present. Check that the file is readable, the sibling is inaccessible and a commit succeeds.
- On Windows with confinement enabled, verify that the ACL grants read access to the generated file and keeps the directory
  and sibling files protected. Exercise the launcher for terminal and structured agents on platforms that support them.

## Implementation notes

- 2026-11-22 (#933): each launch's file lasts as long as the launch. The node keeps it with what the launch started (a terminal
  agent's handle, an adapter agent's entry) and removes it when that process ends, is stopped or is replaced by a restart,
  which writes a fresh file of its own. A start that fails after the file was written removes it. At its start, after taking
  up what the terminal holder kept, the node removes the `agent-<32 hex>` files (and their `.new` staging copies) in its own
  `git-config/` folder that no agent of it uses. An agent the holder keeps is never confined (ADR-0079), so it has no file
  there. Other names in that folder and the legacy copies of decision 4 are not touched.
