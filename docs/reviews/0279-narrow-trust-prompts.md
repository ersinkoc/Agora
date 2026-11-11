# Narrow trust prompts on main (#279)

PR #261 was merged into `claude/detect-wrapped-phrases`, after #259 had landed on main.
Its merge `9a2743d` is not an ancestor of main at `6f35364`. Main therefore had the
whitespace-aware matcher but lacked the sized captures and the short-screen Codex rule.

This change brings commit `d563516` onto main without changing its six code/manifest/fixture files.
The macOS cross-review on #261 checked these exact changes, passed the shipped-manifest test,
and verified Claude's trust prompt at both sizes on a real macOS node. The captures are from
Windows; this round replays them on Ubuntu and does not launch real agents.

## Evidence

- With the sized harness/captures and main's Codex manifest, `cargo test -p agora-adapters
  --test shipped_manifests -- --nocapture` fails: `trust@24x20.screen` detects `none`
  instead of `Blocked trust_prompt`.
- Restoring the reviewed manifest makes both test cases in `shipped_manifests` pass,
  including the 120x40 and 24x20 Claude/Codex trust captures and all existing states.
- `cargo test -p agora-adapters`, `cargo fmt --all -- --check`,
  `cargo clippy -p agora-adapters --all-targets -- -D warnings`, and
  `sh scripts/check-docs.sh` pass on Ubuntu.

There is no runtime, protocol, dependency, or detector hot-path change. Terminal size comes
from the fixture filename in the test harness. A short Codex terminal can still hide its
menu; enlarge it to see the options. Detection does not grant trust or answer the prompt.
