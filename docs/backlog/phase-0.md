# Phase 0 — Bootstrap

Goal: a repository where humans and agents can work in parallel safely.

### P0-01 Maintainer decisions
- **Owner:** human
- **Do:** accept or revise ADR-0009, 0010, 0011, 0016 (license, plus CLA/DCO). Pick final name or confirm working name.
- **Done when:** those ADRs are `Accepted` or superseded; `LICENSE` file exists.

### P0-02 Repository settings
- **Owner:** human
- **Do:** create GitHub repo, branch protection on `main` (PR required, CI required, no direct pushes),
  sync labels from `.github/labels.yml`, enable private security advisories.
- **Done when:** a test PR cannot merge with failing CI.

### P0-03 CI hardening
- **Crates:** none (CI only)
- **Do:** add caching, `cargo deny check`, pin toolchain version in `rust-toolchain.toml`, add a release
  build job for Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64.
- **Done when:** CI green on all three OSes; release artifacts uploaded for a tag in a fork.

### P0-04 i18n lint
- **Crates:** `agora-i18n`, scripts
- **Do:** a check that every Fluent key referenced in Rust code exists in `locales/en/*.ftl`, and that no
  other locale has keys missing from `en` (warnings) or extra (errors).
- **Done when:** `just check` fails on a missing key in a test case.

### P0-05 Fake agent skeleton
- **Crates:** `agora-fake-agent`
- **Do:** a binary that plays a script (TOML) of screen frames and delays to a terminal, so detection
  can be tested. ACP and adapter modes come later.
- **Done when:** `agora-fake-agent --script x.toml` reproduces a working → blocked → idle sequence.

### P0-06 Benchmark harness skeleton
- **Crates:** new `bench/` workspace member (or `agora-fake-agent` extensions), CI
- **Read:** ADR-0022, `design/performance.md` §3
- **Do:** `just bench` runs named scenarios; first scenario: fake agent producing 10 MB/s through a PTY into
  the VT backend, reporting throughput and p95 latency; JSON results stored per commit in CI artifacts.
- **Done when:** a deliberately slowed code path makes the CI comparison fail.
