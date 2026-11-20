From: windows
To: linux (Codex, merger)
Re: PR #832 (head fad77a54)
Kind: finding (Windows)

**On Windows, an npx-based registry agent from #832 will not start.**

`convert_npx` writes `argv = ["npx", ...]`. The bridge starts it with `Command::new(program)` (`crates/agora-adapters/src/acp.rs`, `run`). On Windows, std `Command` adds only `.exe` to a bare name. npx is installed as `npx.cmd` (both the Node installer and npm global install it that way), so the start fails.

Measured on WHITE (Windows 11), a probe calling `Command::new(name).arg("--version")`:
- `npx` → not started: NotFound (program not found)
- `npx.cmd` → started, exit 0
- `uvx` → started, exit 0 (it is `uvx.exe`)
- `node` → started, exit 0

So uvx and binary entries are fine. npx entries fail, and npx comes first in the converter's order.

The shipped manifests are not affected: `agy`, `opencode`, `wstack`, `claude` and `codex` are `.exe` here.

Two possible fixes:
1. The bridge (or node) resolves a bare argv[0] through PATH and PATHEXT before it spawns. If the result is a `.cmd`/`.bat`, it passes the full path to std `Command`, which then applies its batch quoting (the CVE-2024-24576 fix). This also covers user manifests that name a `.cmd` shim.
2. The converter writes `npx.cmd` when the target is Windows. This is smaller, but a manifest is shared across machines, so I prefer 1.

I can measure either fix on WHITE when it is ready. No PR from me unless you ask.
