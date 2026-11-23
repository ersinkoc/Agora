# Agent package list (ADR-0087, rungs 2 and 3)

A signed list says which agent programs (Claude Code, Codex, a Node runtime, ...) a node may offer to install, and how. This is the
**data and its verification** (rung 2); rung 3, at the end of this page, installs one entry kind from the command line. No protocol
message and no schema change. Code: `crates/agora-adapters/src/agent_packages.rs`. Fixtures: `crates/agora-adapters/tests/fixtures/agent-list/`
(a FAKE list signed by a public seed; fixtures are only added, never edited).

## The envelope is ADR-0044's
The file is the same envelope as the adapter index: `{"payload": "<json text>", "sig": "<hex Ed25519 over the payload bytes>"}`.
`registry::open_signed` is the one check for both lists: signature under the root key pinned on the node, `now < expires_ms`, and
`version >= last seen` (rollback). The first-party list is signed by the release key of ADR-0019/0034; a third-party list is a source added
only on the node (`agora agent source add`, a later rung), never from the hub. The envelope text is at most 1 MiB.

Payload (unknown fields refuse the list; so do duplicate publisher ids, unknown publisher fields, and the same key twice anywhere inside an entry, because two readers of a signed entry must see the same entry):
```json
{ "version": 7, "expires_ms": 2000000000000,
  "publishers": [ { "id": "official", "key": "<64 hex>", "revoked": false } ],
  "agents": [ <entry>, ... ] }
```
At most 256 publishers and 256 entries. A revoked publisher's entries load (so the owner can be told why) but `AgentList::offer`
refuses them (`agent.install.revoked`). An entry naming a publisher the list does not declare refuses the whole list.
**One invalid entry refuses the whole list**, with the entry's index, name and a reason code: a signature says who wrote an entry, not
that it is safe to act on. Two entries with the same `name` and installer kind refuse the list; one `name` may have entries of different
installer kinds (the node picks by platform in a later rung).

## Entry
One entry per agent program and installer kind. Unknown fields are refused. `name` is the agent kind and must equal a manifest's `kind`
(`AgentList::without_manifest` lists the ones that do not; the entries are private to `AgentList`: read them with `names()` and `offer()`, which is the revoked-publisher gate; the user-level Node entry is the expected exception, decided by the caller).

| Field | Rule |
|---|---|
| `name` | `a-z`, `0-9`, `-`, `_`, at most 64 (the same rule as adapter names) |
| `display` | 1 to 80 characters, no control, format, bidirectional, separator or private-use character |
| `changelog`, `homepage` (and every download URL) | `https://` only, at most 500 bytes; the host is a DNS name of at least two labels (`a-z`, `0-9`, `-`), whose last label is not all digits nor `0x` and hex digits (`127.0.0.0x1` is an address), and not `localhost`, `*.local`, `*.internal`; no IP literal, no userinfo, an optional port 1 to 65535; no whitespace, quote, backslash, angle bracket or non-ASCII. **This is syntax only.** Rung 3, which downloads, MUST also refuse a name that resolves to a private, loopback or link-local address, validate the port again, and refuse a redirect to a non-https scheme or to another host |
| `publisher` | an id declared in `publishers` |
| `version` | the one exact version this entry installs: `major.minor.patch`, optional `-pre`/`+build`; no tags, ranges or carets |
| `version_policy` | `pin`: exactly `version`. `track`: `version` is the newest the list names (a new list moves it). `range`: also `min` and `max` (exact), and `min <= version <= max` compared on major.minor.patch. `min`/`max` on `pin` or `track` are refused |
| `provides` | 1 to 4 executable names left on the agent's path: `A-Za-z0-9._-`, no leading `.` or `-`, never a shell or interpreter name under any extension (`.exe`, `.cmd`, `.bat`, `.ps1`, `.com`, case-insensitive, repeated: `node.exe`, `pwsh.ps1`, `bash.exe.bat`), and the same rule applies to `cmd` and to the converter |
| `verify` | `{ "args": [...], "regex": "..." }`. `args`: at most 4, each at most 32 characters of `A-Za-z0-9._=-` (so `--version` is fine; a space, quote, `$`, `;`, `|`, `&`, `/` is not). `regex`: at most 200 bytes, compiled by the `regex` crate (linear-time, no backreferences) with a 100 000-byte compile limit and nest limit 20; a pattern that matches the empty string is refused because it would verify nothing. A pattern that matches any non-empty output is still allowed, so a `verify` match alone proves little: a passing `verify` also requires the executable named in `provides` to exist in the agent directory (rung 3 checks both). The version output is matched on its first 4096 bytes only |
| `installer` | one of the four below |

Installers (a closed set; an entry never carries a script the node runs):
- `npm-global`: `package` (an npm name, `name` or `@scope/name`, no URL, git, alias or tag syntax) and `integrity` (`sha512-` and 86
  base64 characters then `==`). The version is the entry's `version`.
- `binary-download`: `platforms`, a map from `<os>-<arch>[-musl]` (`linux-`, `darwin-` or `windows-` first; at most 16) to
  `{ url (https), sha256 (64 lower-case hex), cmd? , signature_url?, signature_key? }`. `cmd` is a relative path in the archive (no `..`,
  no drive, no absolute path, never a shell or interpreter). `signature_url` and `signature_key` (a name) come together or not at all.
- `package-manager`: `manager` in `brew`, `scoop`, `winget` (user-level modes only; others are not in the set) and `package` (letters,
  digits, `._-/@+`, no `..`, no leading `-`).
- `vendor-script`: `url` (https), `sha256` and `interpreter` in `sh`, `powershell`. There is no form without the hash.

Stable codes (`agent_list.*`): `not_object`, `unknown_field`, `missing_field`, `bad_name`, `bad_text`, `bad_url`, `bad_version`,
`bad_policy`, `bad_installer`, `bad_hash`, `bad_token`, `bad_regex`, `too_many`, `bad_platform`, `bad_command`, `forbidden_executable`.
They are the machine detail for `agent.install.index_invalid` when a rung that shows them is built; no window text exists yet.

## The converter (ADR-0092) and this format
`registry_convert` writes a **draft** package file per registry agent. It is reviewed in a pull request, and a reviewer turns it into
entries of the signed list. The two do not fork; the differences, all deliberate:

| Converter draft | Signed-list entry |
|---|---|
| `installers`: an array in one file | one entry per installer kind (same `name`) |
| `binary-download` with `platforms` of `{url, sha256, cmd, env_names_not_copied}` | the same installer; `env_names_not_copied` is a review aid and is dropped; a platform without a `sha256` (`installable = false`) is left out, never guessed |
| `npx` and `uvx` installers (`installable = false`, `no_integrity_hash`) | `npm-global` needs the registry's `integrity` value, added at review (`npx --yes` is the *launch* the L2 manifest keeps, not the install); `uvx` has no installer kind yet, so such an agent is "not offered here" |
| `args` (the ACP arguments) | not in the entry: they belong to the manifest's `[structured] acp.argv` |
| `name = registry-<id>`, `tested`, `source`, `skipped`, `_generated` | `name` is the manifest `kind` (so `registry-<id>`); `source.website` becomes `homepage`; the rest stays draft-only |
| (none) | added by review: `publisher`, `changelog`, `version_policy`, `provides` (the converter's `provides` is reused), `verify` |

`tests/agent_packages.rs::converter_output_fits_the_entry_format_after_review` runs a real converter fixture (`goose`) through this
parser to keep the two aligned.

The drafts for the whole registry are committed as `manifests/agents/registry/<id>.package.json` (unsigned; no release step signs an
agent list yet, so nothing in the repository is a signed entry). Two shapes of real entries: uvx's own `name@version` pin is read as
`name==version`; a `binary` whose program has a different name on each platform (`tool-linux-amd64`, `tool-windows-arm64.exe`) gets
`provides = [<registry id>]`, the one name the installer leaves on the path for all of them, and the manifest runs that name (the
arguments must still be the same on every platform, else `registry.inconsistent_command`). An entry the registry's own
`quarantine.json` names is skipped (`registry.quarantined`), and a `repository`, `website` or `license_url` that is not an `https://`
address under the URL rule above refuses the entry (`registry.bad_url`).

## Rung 3: `agora agent install` (binary-download only)
`agora agent install NAME --list FILE [--root-key HEX|FILE] [--go-back]` installs one agent program on the machine the owner is
typing at. Code: `crates/agora-adapters/src/agent_install.rs` (the logic, with the network and the signature tool behind traits) and
`crates/agora/src/agentcmd.rs` (the words and the question). Nothing here involves the hub, a window or a protocol message.

1. **The list.** Read from `--list`, opened with `verify_list` under the root key (`--root-key` as 64 hex characters or a file; default
   `<AGORA_HOME>/agent-list-root-key`). The newest accepted version is kept in `<AGORA_HOME>/agents/list-state.json` (`{"version":N}`,
   replaced only by a rename so a crash leaves the old file whole, only goes up), so an older list is refused as a rollback
   (`agent.install.index_invalid`, detail `rollback`). The version is recorded under the install lock after the record is read
   again there: a list checked against an older record is refused as `rollback` when another install recorded a newer list
   meanwhile, and the same version is already recorded. The check fails closed (#923): only a missing file means "no list accepted
   yet" (the list is accepted and its version recorded). The file is opened without following a link at its last path component
   (`list-state.json` itself) and checked on the opened handle. Recording a version opens the `agents` folder once (a link or
   junction there is refused) and does everything else through that held handle (`agora_platform::ConfinedDir`, #1000): the first
   read, the install lock (`agents/.lock`), the read again under it, and the replace (a new private file beside the record, renamed
   over it), so all of them address the same folder even if another folder is put at its path meanwhile. Not covered: the folder is
   opened by path that one time (a link checked and then opened, as in item 7), the check before an install
   (`last_seen_version`) is one read by path, the removal of `.lock.stale-*` names left by older versions goes by path, and on
   Windows the lock file is opened at the held folder's present path and then refused when it is not inside the held folder (an
   empty file may be left in a folder put there) and a replace fails while another process has the record open. A file that is there but cannot be read or understood (no permission, a folder or link at that path, larger than
   4 KiB, not a JSON object with exactly one whole-number `version`) refuses **every** list, older or newer, with
   `agent.install.index_invalid`, detail `list_state_unreadable`, and is never overwritten. The owner recovers by removing the file
   and running the install again with the newest list from its publisher, which is then accepted as the first one; the message says
   so and names the file. Removing it is the owner's own decision on their own machine, the same trust as writing it.
2. **The plan.** `AgentList::offer` (a revoked publisher: `agent.install.revoked`), the entry's `binary-download` installer, and the
   key of this system (`agora_platform::platform_key()`: `linux-x86_64`, `darwin-aarch64`, `windows-x86_64`, `-musl` on a musl
   build); none: `agent.install.unsupported_platform`. The node prints the exact action from its own state: name, version, URL,
   SHA-256, publisher and key, signature key if any, the target folder, and the version check it will run.
3. **The approval is typed.** The owner types the agent name, exactly, on a terminal. There is no `--yes`; a stdin that is not a
   terminal refuses (`agent.install.not_approved`). Going to a version lower than the installed one (ADR-0087 item 9) is refused
   (`agent.install.downgrade`) unless `--go-back` is given and the owner also types the version.
4. **The download.** The address is `https://` with a DNS name, checked again (port 1 to 65535, no userinfo, no IP literal). The host
   is resolved by Agora first; one address that is loopback, private, link-local, unique-local, CGNAT (`100.64/10`), unspecified,
   multicast, documentation, benchmarking or reserved refuses the name, and IPv6 forms that carry an IPv4 address (mapped, NAT64,
   6to4, compatible) are judged by the address inside. `curl` is told to connect only to the checked address (`--resolve`), to ignore
   `.curlrc` and proxies, to speak only https, and to follow **no redirect** (a 3xx fails with `download_failed`); the body is read
   by Agora through a pipe, capped at 200 MiB, and hashed on the way (`agent.install.download_failed`, detail `too_large`).
   `curl` missing is `agent.install.tool_missing`.
5. **The checks.** SHA-256 equals the entry's (`checksum_mismatch`). With `signature_url` and `signature_key`, the detached signature is
   fetched the same way (64 KiB cap) and checked with `ssh-keygen -Y verify -I agora-agent -n file` against
   `<AGORA_HOME>/agent-keys/<signature_key>.pub` (an allowed-signers file naming the principal `agora-agent`); no such file refuses
   before any download (`signature_invalid`, detail `key_missing`).
6. **Format.** Only a raw executable: the download is the program, named by the entry's single `provides` name (`.exe` added on
   Windows). An entry with `cmd` (a program inside an archive) is refused: `agent.install.unsupported_format`. This tree has no
   inflate code and the rung adds no dependency, so `.tar.gz` and `.zip` are a later rung, with the restrictive reader of ADR-0087 item 2.
7. **Layout.** `<AGORA_HOME>/agents/<name>/<version>/<exe>` (folders `0700`, program `0700` on Unix; a protected ACL on Windows),
   `<AGORA_HOME>/agents/<name>/current` (the installed version), and the launcher `<AGORA_HOME>/agents/bin/<exe>`: a relative symbolic
   link on Unix, a copy on Windows; `<name>/exe` records which launcher name an agent owns (another agent is refused that name, and
   an agent cannot be named `bin`). The `agents` folder itself must not be a link or junction (refused); it is opened once and the
   temporary folder, the staged program, the signature file and the version-folder checks go through `agora_platform::ConfinedDir`
   (a link is refused, never followed), as do the install lock and `list-state.json` (item 1). The later steps (the rename into
   place, the launcher, `current`, the removal of old versions) use ordinary paths inside that private folder, under the install
   lock; they are not handle-relative.
   The folder is made in `.tmp-<pid>` and renamed into place; nothing in `bin` changes until `verify` passed.
8. **Verify, then switch.** The new program is run with the entry's `verify` arguments, 10 seconds, stdin closed, an empty environment
   but `PATH=/usr/bin:/bin` (system folders on Windows) and a home inside its own folder (no token or secret of Agora reaches it); the
   first 4096 bytes of its output (standard output then standard error) must match the entry's regex and it must exit with success,
   and the `provides` program must exist (`verify_failed`, `timeout`). Only then the launcher moves (atomic rename) and `current` is
   written; if either step fails the previous launcher is put back first. A failed verify removes the new folder and leaves the old
   launcher. A folder the launcher resolves into is never removed. The new version and the one before it are kept; older
   ones are removed.
9. **One at a time, and crashes.** `<AGORA_HOME>/agents/.lock` is held with the system's file lock (`File::try_lock`; opened through
   the held `agents` folder with `ConfinedDir::open_lock`, the checks of `agora_platform::open_lock_private`), which the system releases when its owner ends, so a crash never leaves it held and nothing
   is taken over (audit F100: the earlier rename-and-check takeover let two installs hold it at once). A lock held by another install
   is `agent.install.in_progress` after a wait of at most one second (a process another thread is starting holds a copy of the
   handle until its exec). While held, the file holds `pid start boot` of its owner (a reused pid does not look alive) for older
   versions, which judge the lock by that text alone; a live owner named there (an older version holding it) is also
   `agent.install.in_progress`, and empty or unreadable text never blocks. On release the file is emptied, never removed. On
   Windows the file is opened without delete sharing, so while any process has it open it can be neither renamed nor removed. Limits
   while versions are mixed: an older version answers `agent.install.in_progress` at once for 60 seconds after a newer one released
   the lock (it reads the young empty file as somebody still writing); on Windows an older version cannot read a held lock (the
   system lock is mandatory there) and judges it by its age, so after 60 seconds it tries to take it over, but its rename fails while
   the newer one holds the file open and it answers `agent.install.in_progress`; elsewhere it reads the owner text. An older version
   taking a fresh lock in the instant a newer one also does is not excluded (on Linux and macOS it can move the file a newer one has
   just opened). An older version that ended between putting a lock back and removing the name it had moved it to leaves `.lock`
   with a second name, which the private open refuses: the `.lock.stale-*` names are then removed and the open is tried once more.
   A home on a network file system gets what that system's locks give: on Linux NFS a file lock is per process (two installs in one
   process would not exclude each other; each `agora agent install` is its own process) and needs the server's lock service; on
   macOS a file system without these locks makes every install fail with `agent.install.io_failed`. The list version is remembered
   under the same lock. Every run first removes `.tmp-*` folders, `.lock.stale-*` files older versions left and half-made launchers
   a crash left.
10. **Never** a script from the entry, `sudo`, a shell profile edit, or a write outside `<AGORA_HOME>`.

Test seams: `Tools` carries the resolver, the fetcher and the signature checker; the command line always passes the strict ones
(`SystemResolver`, `CurlFetcher`, `SshKeygen`). The address rules run on the answers of whatever resolver is passed.

## Used by the node and shown by doctor (ADR-0087 items 3 and 6)
A program `agora agent install` put under `<AGORA_HOME>/agents/bin/` is the one the node starts for a kind whose manifest `launch`
(terminal agents) names it, and `agora doctor` reports it as installed "by Agora" with its version, against "by you" for a program found
only through the user's `PATH` or a full path. The lookup (`agent_install::installed_program`) happens before the user's `PATH` and
accepts a program only when all of these hold; otherwise the `PATH` decides as before:
- the manifest's program is a plain file name (no separator, not `.` or `..`); on Windows `claude` also matches the installer's `claude.exe`;
- an agent folder's `exe` file names it, and that agent's `current` file names a version;
- `agents`, `agents/bin` and (Unix) the version folder are real folders (not links, not junctions), this user's, and not writable by group or others;
- the launcher is what the installer writes: on Unix a link that resolves to `agents/<name>/<current version>/<program>`, on Windows a
  regular file in `bin`; a link to anywhere else (another agent, an old version, outside `agents`) is refused;
- on Unix the resolved file belongs to this user and is not writable by group or others.
When a folder owns the name but a check fails, the node writes a line to its log (the program's file name only) and starts the one on the `PATH`; `agora doctor` adds an "installed by Agora, but not safe" note (`agent-unsafe`).
The node then starts the resolved real file by its full path. The agent's own `PATH` is not changed: it does not gain
`agents/bin`, so an agent that needs a sibling program installed the same way will not find it there. Agents started through ACP or an
adapter command still resolve their program through the user's `PATH`.

## Not here
No archives (rung 3 follow-up), no other installer kind (`npm-global`, `package-manager` and `vendor-script` are rung 6), no signed
device approval, schema message, hub involvement or window (rungs 4 and 5), no `agora agent source add`, no refresh of the list. The
node does not put `agents/bin` on an agent's `PATH`, and ACP and adapter commands are not looked up in `agents/bin` yet.
