# Agent package list (ADR-0087, rung 2)

A signed list says which agent programs (Claude Code, Codex, a Node runtime, ...) a node may offer to install, and how. This is the
**data and its verification only**. Nothing reads the network, installs or runs anything yet (rungs 3 and later); no protocol message
and no schema changes. Code: `crates/agora-adapters/src/agent_packages.rs`. Fixtures: `crates/agora-adapters/tests/fixtures/agent-list/`
(a FAKE list signed by a public seed; fixtures are only added, never edited).

## The envelope is ADR-0044's
The file is the same envelope as the adapter index: `{"payload": "<json text>", "sig": "<hex Ed25519 over the payload bytes>"}`.
`registry::open_signed` is the one check for both lists: signature under the root key pinned on the node, `now < expires_ms`, and
`version >= last seen` (rollback). The first-party list is signed by the release key of ADR-0019/0034; a third-party list is a source added
only on the node (`agora agent source add`, a later rung), never from the hub. The envelope text is at most 1 MiB.

Payload (unknown fields refuse the list):
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
(`AgentList::without_manifest` lists the ones that do not; the user-level Node entry is the expected exception, decided by the caller).

| Field | Rule |
|---|---|
| `name` | `a-z`, `0-9`, `-`, `_`, at most 64 (the same rule as adapter names) |
| `display` | 1 to 80 characters, no control, format, bidirectional, separator or private-use character |
| `changelog`, `homepage` | `https://` only, at most 500 bytes, a host with no userinfo, no whitespace, quote, backslash, angle bracket or non-ASCII |
| `publisher` | an id declared in `publishers` |
| `version` | the one exact version this entry installs: `major.minor.patch`, optional `-pre`/`+build`; no tags, ranges or carets |
| `version_policy` | `pin`: exactly `version`. `track`: `version` is the newest the list names (a new list moves it). `range`: also `min` and `max` (exact), and `min <= version <= max` compared on major.minor.patch. `min`/`max` on `pin` or `track` are refused |
| `provides` | 1 to 4 executable names left on the agent's path: `A-Za-z0-9._-`, no leading `.` or `-`, never a shell or interpreter name |
| `verify` | `{ "args": [...], "regex": "..." }`. `args`: at most 4, each at most 32 characters of `A-Za-z0-9._=-` (so `--version` is fine; a space, quote, `$`, `;`, `|`, `&`, `/` is not). `regex`: at most 200 bytes, compiled by the `regex` crate (linear-time, no backreferences) with a 100 000-byte compile limit and nest limit 20; a pattern that matches the empty string is refused because it would verify nothing. The version output is matched on its first 4096 bytes only |
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

## Not here
No download, install, approval, schema message or window. No `agora agent source add` and no refresh of the list. The doctor of
rung 1 does not read this list yet.
