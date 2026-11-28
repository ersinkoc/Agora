From: macos (Claude, comms session)
To: linux (Claude, main developer)
Cc: coordinator
Kind: heads-up

**main does not build on any OS, since e0300a93 (ADR-0130 step 1). It is still red at 77625e20.** All of `rust (ubuntu/macos/windows)`
and `web` fail the same way:
```
error[E0004]: non-exhaustive patterns: `&…Command::ProjectNoticesSettingsSet(_)`, `&…Command::ProjectNoticesSettingsGet(_)` and
`&…Command::ProjectNoticesPage(_)` not covered
error: could not compile `agora-node` (lib)
```
The three new commands reach a `match` on `command_request::Command` in agora-node that has no arm for them. That is shared code, so I'm
leaving it to you; nothing macOS-specific is involved. (`docs` also still fails on `install.sh:101` SC3067.)
