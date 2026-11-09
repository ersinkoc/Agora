# 0044. Installing adapters and manifests from the registry
Status: Accepted
Date: 2026-11-09

## Context
ADR-0018 defines a signed registry (index, publishers, packages) and `agora-adapters::registry` verifies an index and a package. It
says "nodes verify signatures before installing or running adapters" and "installing third-party adapters requires explicit user
approval", but nothing fetches, stores or installs anything: no URL layout, no package layout, no install directory, no command.
Phase 6's exit ("a third-party adapter in another language connects without a core change") works today by hand-placing a manifest
with an `[adapter]` command; the registry is what makes it safe for someone else to do it.

## Decision
1. **Layout.** A registry is a base URL with `index.json` (the signed envelope `verify_index` takes) and, per package entry,
   `packages/<name>-<version>.tar` plus `packages/<name>-<version>.sig` (hex Ed25519 over the tar bytes). A package is a plain tar
   (no compression, no links, no absolute or `..` paths, at most 64 files and 64 MiB unpacked) holding `manifest.toml` and, for an L3
   adapter, the files its `[adapter] command` runs.
2. **Where it lands.** `<AGORA_HOME>/adapters/<name>/<version>/`, owner-only. The manifest is copied to `<manifests dir>/<name>.toml`
   with `[adapter] command` entries that start with `./` rewritten to that directory; nothing outside these two places is written.
3. **Command.** `agora adapter install NAME [--registry URL] [--root-key FILE]`, `agora adapter list`, `agora adapter remove NAME`.
   The root key is pinned like the release key (`<AGORA_HOME>/registry-root-key`); the last index version seen is stored beside it
   for the rollback check. There is no default registry: the user names one, or the hub's `<hub dir>/registry-url` is used.
4. **Approval.** The install prints publisher, name, version, hash and the command the adapter will run, and asks once (`--yes`
   skips for first-party publishers only). A package from a publisher the user has not approved before makes the publisher's id and key
   part of that question; approved publishers are remembered in `<AGORA_HOME>/publishers`.
5. **Nothing runs during install.** Installing unpacks and registers; the adapter starts only when an agent of its kind is started.
6. **Revocation and rotation.** A revoked publisher refuses new installs (already enforced by `verify_package`); installed packages
   of a publisher that is revoked in a newer index are reported by `agora adapter list` and not started until the user removes or
   re-approves them. Key rotation is a new publisher entry in a signed index.
7. **Downloads** use `curl` like `agora update`, over HTTPS only unless the address is loopback.

## Consequences
Adding agent support needs no binary release, and a hostile mirror can neither add nor swap code. The user stays the authority on what
runs as them (ADR-0018). Windows and macOS install the same layout. The tar reader is the only new parsing code and is deliberately
restrictive; a dependency-free reader is about 100 lines.
