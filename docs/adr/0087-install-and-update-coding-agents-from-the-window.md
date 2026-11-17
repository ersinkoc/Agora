# 0087. Install and update coding agents on a machine from the window
Status: Proposed
Date: 2026-11-17

Not accepted. Nothing here may be built until the maintainer names this ADR in an acceptance message.

Builds on ADR-0020 (signed approvals), ADR-0034 (install a node over ssh), ADR-0044 (the signed registry), ADR-0067 (remote access),
ADR-0062 (`agora up`), ADR-0069 (a missing agent kind says why), ADR-0075 (the node updates in place). Touches ADR-0004, ADR-0018,
ADR-0057 and ADR-0084 and the threat model.

Origin: the maintainer's wish: install Agora on a VPS, reach it remotely (Tailscale, Cloudflare or a plain address, behind the screen
password of ADR-0067), and then, from the window, install and update Claude Code, Codex, WrongStack and the other agents on that machine,
without opening an ssh session for each one.

## Context
- Today an agent kind is a manifest (ADR-0004). When the program is not on the machine, the window says so (ADR-0069) and stops. The
  owner installs the agent by hand, in a terminal on that machine, and keeps it current by hand. On a fresh VPS that is the longest part
  of the setup, and on three machines it is three times the same chore.
- ADR-0044 installs *adapters and manifests* from a signed registry. It does not install the agent programs themselves: those come from
  their own vendors, through an npm package, a vendor script, a binary download or an OS package manager.
- Installing a program that then runs as the user, with the user's files and, later, the user's agent login, is the most sensitive thing
  the window could ask a node to do. A hub compromise (or a stolen session) that could push an install would own every machine. ADR-0020
  already names it: "Starting a new agent kind or installing an adapter on a node is a sensitive action." This ADR designs the same rule
  for installing the agent program itself.

## Decision
Built in small rungs (see Build order). The shape:

1. **An agent package is data.** A signed list of *agent packages*, one entry per agent program and per installer kind. It reuses the
   ADR-0044 envelope: a signed index with publishers, a root key pinned on the node, a rollback check on the index version, a revoked
   publisher refuses new installs. An entry holds:
   - `name` (the agent kind, equal to the manifest's `kind`), `display` name, a `changelog` link and a `homepage`;
   - `installer`, one of the closed set below, with its fixed parameters;
   - `version_policy`: `pin` (one exact version), `track` (the newest the entry's `latest` field names, which the signed index says),
     or `range` (a lowest and a highest; the node refuses outside it);
   - `provides`: the executable names the install must leave on the path (`claude`, `codex`), and `verify`, the argument list that prints
     the version (`--version`) with a regular expression the output must match.
   Adding a new agent is an entry plus a manifest: no change in a core crate (ADR-0004, "agents are data").
2. **The installer kinds are a closed list the node implements.** A package names a kind; it never carries a script the node runs.
   - `npm-global`: `npm install --prefix <agent dir> PACKAGE@VERSION` into a directory the node owns, with the
     package name, the exact version and the registry's integrity hash (`sha512-...`) in the entry. The node verifies the integrity of what
     was downloaded before it links the executable. It never uses `-g` into a system prefix.
   - `binary-download`: an HTTPS URL per OS and architecture, a SHA-256 in the entry, and optionally a detached signature the entry
     names a key for (the same ssh-signature check as the release in ADR-0034). The node downloads to a temporary file in the agent
     directory, checks the hash (and the signature when given), then unpacks with the same restrictive reader as ADR-0044 (no links, no
     absolute or `..` paths, size limits).
   - `vendor-script`: allowed only as a *pinned script*: the entry holds the script's URL **and** its SHA-256; the node downloads the file,
     compares the hash, and only then runs it, with the user's privileges, a scrubbed environment, a home pointing at the agent directory
     where the script allows it, and a time limit. A URL without a hash is invalid and the index is refused. There is no `curl | sh`.
   - `package-manager`: the OS package manager's *user-level* mode only, with a package id and version; used where the vendor ships only
     that way. If the manager would need administrator rights the node does not try (see 7).
3. **Where it lands.** `<AGORA_HOME>/agents/<name>/<version>/`, owner-only, and a link `<AGORA_HOME>/agents/bin/<exe>` that the node puts
   on the *agent's* path when it starts that kind (the manifest's `launch` resolves there first, then the user's path). Nothing outside
   `<AGORA_HOME>` is written, no shell profile is edited, nothing is put in `/usr`. An agent the user installed themselves elsewhere is
   untouched and is shown as "installed by you".
4. **The approval is the gate (ADR-0020).** The hub relays a request "install NAME" from a window; the node does nothing until it holds
   an approval **signed by a trusted device of the owner** and verifies it against `trusted-devices` on the node. The signed text is the
   node's own rendering of the exact action, never the hub's: the package name, installer kind, the exact version, the source (URL or
   package name and registry), the checksum or integrity value, the publisher and its key id, the target directory and the command
   argument list. The window shows what the node returned, signs it, and the node refuses if what it is about to run differs from what
   was signed by one byte. The approval is single use, bound to the machine, the entry's content hash and an expiry (minutes). `--yes` or
   a remembered "always allow" does not exist for the first install of a kind; an *update* inside the same publisher and the same
   `range` may be remembered per agent, and the node policy can switch that off.
5. **The agent's own login stays on the machine (ADR-0084).** Installing does not log in. The window may open a terminal on that machine
   running the manifest's `accounts.login` so the user types the code or key there; the hub carries terminal bytes as for any terminal
   (ADR-0022) and stores nothing. The hub never holds an API key or token, the install request has no field for one, and the install
   output shown in the window is filtered like any terminal output. A stored secret for the agent is the owner's own
   `agora node secret set NAME` on the node (ADR-0084), not something the window sends.
6. **Doctor and the window.** `agora node doctor` (and the same data on the machine page) lists, per agent kind in the node's manifests
   and the signed list: *installed* (with the version and "installed by Agora" or "installed by you"), *missing*, *outdated* (the list's
   newest allowed version is higher), *newer than allowed* (above a `range`), *broken* (the executable is there but `verify` fails) and
   *not offered here* (no installer for this OS and architecture). The window uses the UI vocabulary: "Install", "Update", "Up to date",
   never the internal terms. Doctor only reads: no network except the optional index refresh the owner asked for, and it works with
   no hub.
7. **User-level only; no sudo from Agora.** Nothing Agora runs on a node uses `sudo`, `su`, a UAC prompt, or setuid. When an install
   needs rights the user does not have (a system package, a missing compiler, a missing `npm` or Node runtime, a headers package), the node
   reports the code `agent.install.needs_admin` with the *name* of what is missing and the command the owner would run themselves, and
   stops. The owner runs it in a terminal on that machine (the window can open one, as in 5), then presses "Try again". Per OS:
   - **Linux.** Everything under the user's home. `npm-global` needs a user-owned Node and npm: if missing, the node offers a
     `binary-download` entry for a Node runtime in the same signed list rather than touching the system. The node never edits a shell
     profile; `agora` finds the programs through its own agent bin directory.
   - **macOS.** The same layout. A downloaded binary carries the quarantine attribute only if the node's own download did; the node
     does not remove it, and a vendor binary that is not signed and notarized is reported as such, not bypassed.
   - **Windows.** Under the user's profile only; the executable link is a small launcher file or a copy (symbolic links need rights).
     No execution-policy change, no machine-wide install, no administrator prompt. A package that offers only an administrator
     installer is "not offered here".
8. **Update flow.** (a) *Check*: the node reads the signed index (on request and at most once a day, with a clear setting to stop it)
   and compares with what is installed; the hub shows the result, never computes it. (b) *Show*: the update card shows the current and
   the new version, the changelog link from the entry, and the exact action of 4. (c) *Approve*: signed as in 4. (d) *Install*: into a new
   `<version>` directory beside the old one; nothing is changed in place. (e) *Verify*: run `verify` (for example `--version`) with a short
   time limit and match the output. (f) *Switch*: only after the check passes, the agent bin link moves to the new version, atomically.
   (g) *Rollback*: the previous version directory is kept (the last two by default), and "Go back" moves the link to it with a new
   signed approval only when going to a version the entry's `range` still allows; a failed verify rolls back by itself and says so.
   Agents already running keep the old program until they are restarted; the window says "running the old version" and never restarts
   an agent by itself (ADR-0075 principle: do not stop agents).
9. **Never downgrade by accident.** The node refuses an index whose version is lower than the last seen (ADR-0044) and refuses an entry
   whose version is lower than what is installed unless the owner's signed approval says "go back to". A `pin` that the owner set on the
   node wins over `track`.
10. **Failure modes are named and leave things as they were.** Each stops with a stable code and parameters (no user text;
    the localization rule), and nothing is half-installed in the live place: `agent.install.index_invalid` (signature or
    rollback), `agent.install.not_approved`, `agent.install.approval_mismatch`, `agent.install.download_failed`,
    `agent.install.checksum_mismatch`, `agent.install.signature_invalid`, `agent.install.needs_admin`, `agent.install.disk_full`,
    `agent.install.verify_failed`, `agent.install.timeout`, `agent.install.unsupported_platform`, `agent.install.in_progress`,
    `agent.install.revoked`. A crash between download and switch leaves a temporary directory the next doctor run reports and removes.
    Only one install per machine runs at a time. The node being offline mid-install ends the install on the node's own terms
    (ADR-0002: nodes are autonomous); the result is sent as an event when the hub returns. Because the new codes and messages
    are protocol, the schema change is its own ADR-gated step (rung 4).
11. **Mesh messages cannot install.** An agent (or a mesh message) asking for an install, an update, a new package source or a
    policy change is refused with a stable code; the mesh has no tool for it, and a human's signed approval is the only path. The
    content of a mesh message may appear *in the question* shown to the user only as quoted, untrusted text, never as the action.

## Threat model for this decision
Read with `docs/security/threat-model.md`; no row is added now, rows are added by the build.
- **A malicious or mistaken registry entry.** The index and each entry are signed by a publisher whose key the node pinned; the node
  shows the publisher and the exact action and a signed approval is needed. A publisher the user has not approved before makes its id
  and key part of the question (ADR-0044). A first-party list shipped with a release is signed by the release key; a third-party list is
  a source the owner adds on the node, never from the hub.
- **Typosquatting.** The package name, the executable names and the checksum are all in the signed entry; the user never types a package
  name into the window, so there is nothing to mistype. The signed text shows the name and publisher; an entry whose `name` differs
  from the manifest `kind` is refused. Names are compared with what a previous install of that kind used, and a change is shown as such.
- **Downgrade and replay.** Index rollback check; the per-entry version rule of 9; approvals expire and are single use, bound to the
  entry's content hash.
- **Hub compromise.** The hub can ask, but it cannot sign, cannot change the entry the node holds, cannot widen the node policy, and
  cannot make the node fetch from a place the signed entry does not name. At worst it can show the user a wrong *button*; the node's own
  text is what is signed. It cannot read what an install writes beyond terminal bytes the user chose to open. It stores no secret.
- **Stolen session.** A session alone cannot sign (ADR-0020); the remote screen password (ADR-0067) is in front of the session; the
  device key is what the approval needs.
- **Compromised vendor package.** A pinned version with a checksum limits the blast radius to what the maintainers of the signed list
  have reviewed; `track` follows only what the list names as latest, and publishers can withdraw a version (revocation, 8's check).
  Installer scripts that `npm` packages run are a known risk; the confinement of ADR-0047 applies to the install process where the OS
  supports it, with network allowed only for the entry's host, and the node scrubs its environment (no tokens, no stored secrets).
- **The install itself running code.** An installed agent is code from its vendor running as the user. That is the same trust the
  owner gives by installing it by hand; this design adds the signed list, the checksum and the approval, not a sandbox for the vendor.

## Explicitly not done
- No sudo, no administrator prompt, no system package install, no edit of shell profiles or machine-wide settings.
- No `curl | sh` of an unpinned URL, no script supplied by the hub, no binary uploaded by the hub.
- No login, key or token handling: the user logs in on the machine; the hub never holds an agent's credentials.
- No auto-update without approval, no restarting a running agent to apply an update.
- No install triggered by the mesh, by an agent, or by a page of the window without a device signature.
- No uninstall of something the user installed themselves; Agora removes only what it put in `<AGORA_HOME>/agents`.
- No package search or "install anything" box: only entries in a signed list.
- No change to the manifest format (ADR-0004); an L3 adapter still installs by ADR-0044.

## Consequences
- New: a signed agent-package list (a sibling of the ADR-0044 index, same verification code), an installer module in `agora-node`
  (the four kinds, temp directory, verify, switch, keep previous), `agora node doctor` columns, `agora agent install|update|rollback`
  (the same flow the window uses), a protocol message pair for "describe the action" and "result", error codes, locale strings, and a
  machine-page card in the window. Platform differences stay in `agora-platform` (links versus launchers, process limits).
- The protocol is append-only and schema-first: the new messages need the schema step and golden fixtures, an ADR-gated change
  already covered by this ADR once accepted.
- Bandwidth and disk: two versions per agent are kept; doctor reports the size, and ADR-0064 hygiene covers old versions.
- A setup on a fresh VPS becomes: install Agora (ADR-0034), reach it (ADR-0067), open the window, "Install" on each agent, sign on the
  trusted device, log in once in a terminal on that machine.

## Alternatives considered
- **Run the vendor's documented command over ssh from the hub.** Rejected: the hub would be a code distributor and an unpinned script
  runner, and ADR-0020 forbids the hub widening what runs on a node.
- **Let the user paste a command into the window.** Rejected: that is a remote shell with extra steps, and every typo is a typosquat.
- **A registry that stores the agents' binaries.** Rejected: the project would rebuild and redistribute other people's programs;
  the signed list points at the vendors' own artifacts and pins them.
- **Leave it manual.** Still supported and unchanged; this removes the repetition on many machines.

## Open questions (proposed answers stand until the maintainer says otherwise)
1. *Who signs the first-party list?* Proposed: the release key of ADR-0019/0034, so the list ships with a release and `agora update` brings
   a new one; a faster refresh channel waits.
2. *May an update inside a remembered range skip the signature?* Proposed: no on the first run, yes only per agent after the owner
   chose "allow updates for this agent" with a signature, and the node policy can forbid it.
3. *Does the list cover Node itself?* Proposed: yes as a `binary-download` entry, user-level, only because `npm-global` needs it.
4. *Which installer for each agent?* Proposed: the vendor's own recommended way, as long as it can be pinned with a checksum; if a vendor
   offers only an unpinnable script, that agent is "not offered here" until it has a pinnable artifact, rather than weakening 2.
5. *Check frequency.* Proposed: on demand and once a day, off by switch; no check when the machine is metered or the owner turned it off.
6. *How many old versions to keep?* Proposed: two.
7. *Windows without a user-level Node?* Proposed: the list's Node entry; failing that `needs_admin` with instructions.
8. *Third-party lists.* Proposed: added only on the node (`agora agent source add`), with the publisher's key shown and approved like ADR-0044.

## Build order
Each rung ships alone and leaves the system working.
1. **Doctor, read-only.** `agora node doctor` lists installed, missing and broken agents from the manifests, with versions; no network.
2. **The signed list.** The entry format, verification reusing ADR-0044, rollback check, fixtures with a fake list; no install yet.
3. **One installer kind, from the CLI, with a local approval.** `binary-download` into `<AGORA_HOME>/agents`, verify, switch, keep previous,
   tested against a fake agent and a local server; the owner's answer typed on the machine.
4. **The schema step and signed approval.** The protocol messages (describe action, result), the node's rendering of the exact action,
   signature check against `trusted-devices`, single use, expiry, mismatch refused. Golden fixtures.
5. **The window.** Machine page card: Install, Update, Up to date; the signed confirmation; the progress and the failure codes; open a
   terminal for the agent's login. Both themes, screenshots.
6. **The other installer kinds.** `npm-global`, `package-manager`, then pinned `vendor-script`, each with its own tests and review.
7. **Updates and rollback.** The check, the changelog link, the switch and "Go back", running-agent notice.
8. **Per-OS runs.** Linux, macOS and Windows, on real machines, recorded under `docs/reviews/`.
