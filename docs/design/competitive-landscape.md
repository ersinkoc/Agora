# Competitive landscape (research snapshot, 2026-11-17)

Status: research note, not a decision. Nothing here changes an ADR. Every external fact carries the page it came from; "seen"
dates are all 2026-11-17 unless stated. Page content was treated as untrusted data. Where only a blog or aggregator said it, or the
page did not show it, the cell says **unverified**. GitHub star counts are what the repo page showed that day and move daily.

## 0. Headline (read this first)

1. **herdr is real, large and moving fast.** It is not just a TUI multiplexer any more. Its site now describes a native desktop app
   (macOS, Linux, Windows), a background server (`herdr-core`), saved SSH machines in one sidebar, native iPhone/iPad/Android apps
   over plain SSH, helper agents, plugins and a 23-agent detection list. 51.4k stars, Apache-2.0, releases every few days
   (v0.9.5 on 2026-10-14). Agora's vision doc still says multiplexers are "single machine, inferred state, partial multi-machine".
   That row is **out of date for herdr** and must not be repeated in marketing.
2. **The "one window, many machines, phone in your pocket" idea is no longer unique.** herdr, Paseo, Happy, Paradev (ex-Superset),
   Cursor (Remote Control), Claude Code (Remote Control) and Codex (phone + remote SSH) all ship some version of it.
3. **What is still rare and where Agora can be clearly ahead:** (a) approvals signed by a device key and verified on the machine,
   (b) per-agent OS confinement with secrets held by the node, (c) a structured, cross-vendor, cross-machine mesh (messages, board,
   leases, helpers with signed OK), (d) agents surviving a node restart (terminal holder, ledger, resume) and updates in place,
   (e) a hub that cannot widen node policy, (f) one binary, no third-party account or relay in the path.
4. **Where Agora is behind, honestly:** agent coverage (3 terminal manifests + 3 ACP vs 20+), install friction and signed installers,
   no native phone app and no true push, the review-to-PR loop is "not built", pairing/join is "not built", Tailscale/Cloudflare
   rungs not built, no scheduled/issue-driven work, no cloud/ephemeral machines, AGPL core vs Apache/MIT competitors, and almost
   no real-machine evidence ("none recorded here" for most rows in `docs/capabilities.md`).

## 1. Profiles

Format: what it is / how it works / platforms / agents / notable / weaknesses. Source on each line.

### 1.1 herdr (the one the maintainer named)

- **Identity.** Canonical repo `github.com/herdrdev/herdr` (the older `ogulcancelik/herdr` path and several forks/mirrors
  `naaive`, `ttbug`, `Suor` also appear in search; treat `herdrdev` plus `herdr.dev` as canonical, **unverified** which is the origin
  of record). Seen 2026-11-17: https://github.com/ogulcancelik/herdr (shows "single Rust binary", 51.4k stars, 4.2k forks, 482 open
  issues, 26 open PRs, 1,880 commits, **Apache-2.0**) and https://herdr.dev.
- **Pitch.** "The runtime your coding agents live on." A single Rust binary, tmux-style prefix keys plus mouse, runs inside your
  terminal. Source: repo page above.
- **Now also a native app.** herdr.dev says: native app running agents in `herdr-core`, "a small server on your machine", agents keep
  running when the window closes, crashes or updates; app, core and TUI are Apache-2.0; macOS (Apple silicon), Linux (x86_64,
  arm64), Windows (x86_64) desktop; iPhone, iPad, Android. Source: https://herdr.dev (seen 2026-11-17).
- **Install.** `curl -fsSL https://herdr.dev/install.sh | sh`, `brew install herdr`, `mise use -g herdr`, a PowerShell script, a DMG
  for the app. Source: repo page and herdr.dev.
- **Agent state.** Per pane working/blocked/idle. Detection finds the agent process, reads the *live bottom* of the pane and maps
  it with **detection manifests** (bundled, with remote updates from herdr.dev, local overrides in
  `~/.config/herdr/agent-detection/<agent>.toml`); agents with an integration report state themselves; `blocked` is strict
  (known approval/question UI only, otherwise `idle`). Source: https://herdr.dev/docs/agents (seen 2026-11-17). Note: this is the same
  design as Agora's L1 manifests and the "bottom of the buffer only" rule; herdr had it first in public.
- **Agents listed as supported (23 + 5 self-reporting).** Claude Code, Codex, Copilot CLI, Cursor Agent CLI, OpenCode, Pi, OMP, Droid,
  Devin CLI, Kimi Code, Kilo Code, Hermes, Qoder, Qwen Code, Letta Code, MastraCode, Grok CLI, Antigravity CLI, Amp, Kiro, Maki,
  Gemini CLI, Cline; plus Command Code, Crush, Muse Code, Prime Agent that report their own state. Adding a brand-new agent
  "requires a Herdr binary update". Source: https://herdr.dev/docs/agents.
- **Persistence.** Detach/reattach keeps everything. A **server restart kills processes**: layout, cwd and focus are restored, agent
  conversations resume only through native session restore (on by default for agents with integrations); pane screen history is
  experimental and off by default. `--handoff` update that transfers live panes is experimental and unavailable for Homebrew/mise/Nix
  installs. 48 layout snapshots kept. Source: https://herdr.dev/docs/session-state.
- **Multi-machine.** Local plus saved SSH machines in one window, combined agent list, independent reconnects. Source:
  https://herdr.dev/docs/ and repo page.
- **Phone.** App connects **over SSH directly to herdr-core**, no account, no relay; offers hosts from SSH config and Tailscale
  devices; inbox sorted needs you/done/working/idle; you reply by typing into the real terminal (key bar, compose, dictation,
  screenshots); no dedicated "approve" action; the page mentions only an in-app banner, no push with the app closed. Source:
  https://herdr.dev/mobile. Third-party relays/apps exist (`0cv/herdr-mobile-relay` QR pairing + installable web app, "Heeler",
  "C3 Mobile"); **unverified** quality. One 2026 guide still says there is no official mobile app (outdated against herdr.dev).
- **Agent control.** CLI and local socket API; agents can spawn panes, prompt each other, wait until another is blocked; one agent can
  start helper agents and receive reports. Source: repo page, herdr.dev.
- **Plugins.** Local executable workflow plugins with event hooks; marketplace "launching". Source: https://herdr.dev/docs/.
- **Activity.** v0.9.5 (2026-10-14, hotfix), v0.9.4 (2026-10-13), a preview build the same day. Source:
  https://api.github.com/repos/herdrdev/herdr/releases (seen 2026-11-17). Several releases per week.
- **Weaknesses (evidence-based).** No isolation or sandbox story found (wrappers are mentioned only so detection still works);
  no signed approvals; approvals are "type into the terminal"; processes die on server restart; phone = SSH reachability (works on LAN or
  Tailscale, nothing for teams or revocation); no hub, no shared identity, no team/audit, no structured mesh (keystroke-level
  prompting); detection of `blocked` is screen-pattern based; "adding a brand-new agent requires a binary update" (contradicts
  Agora's "agents are data"). Diff/review UI, worktree handling, cost tracking: **not found on the pages I read, unverified**.
  482 open issues suggests maintenance load.

### 1.2 Claude Code's own remote and background features (Anthropic)

- **Remote Control.** Connects claude.ai/code or the Claude iOS/Android app to a session running on your machine; execution stays
  local; outbound HTTPS only, no inbound ports; **transcript is stored on Anthropic servers**; needs Pro/Max/Ultra/Team/Enterprise,
  **not API keys**; Team/Enterprise owner must enable it; "Trusted Devices" option (biometric/passkey after 18 h);
  `claude remote-control` server mode with `--spawn worktree`, 32 concurrent sessions by default; phone push when a task finishes
  or needs a decision. Source: https://code.claude.com/docs/en/remote-control.md and
  https://code.claude.com/docs/en/mobile.md (seen 2026-11-17).
- **Agent view (research preview).** `claude agents` single screen of background sessions, supervisor process, sessions in their own
  worktrees, states working/needs input/idle/completed/failed/stopped, peek and reply, `--json` for scripts. Shutdown stops sessions
  (recent ones restart on reply). Source: https://code.claude.com/docs/en/agent-view.md.
- **Agent teams (experimental, off by default).** Lead plus teammates, shared task list, mailbox files under `~/.claude/teams`,
  tmux/iTerm2 split panes; no resume of in-process teammates, one team per session, no nested teams. Source:
  https://code.claude.com/docs/en/agent-teams.md.
- **Cross-session messaging.** Claude can list and message your other sessions, including other machines, through Remote Control
  (via Anthropic servers) and cloud sessions; plain text only; a message cannot approve anything; inbound controls
  accept/hold/refuse. Source: https://code.claude.com/docs/en/cross-session-messaging.md.
- **Cloud sessions, routines, projects, Desktop with worktrees and Linux beta, Channels, self-hosted environments.** Index at
  https://code.claude.com/docs/llms.txt.
- **Weakness vs Agora.** Claude-only; needs an Anthropic login and sends the transcript through Anthropic; no cross-vendor view; no
  node-verified signed approvals; mobile cannot select bypass mode (a safety plus).

### 1.3 OpenAI Codex (app, mobile, remote)

- Codex desktop app: macOS Feb 2 2026, Windows Mar 4 2026; threads per agent, built-in worktrees; phone preview May 14 2026; remote SSH GA.
  **Secondary sources only** (theneuron, heise, IT Brief via search); OpenAI's own page
  `openai.com/index/work-with-codex-from-anywhere/` returned HTTP 403 to my fetch, so dates are **unverified**.
- Official docs page seen: https://learn.chatgpt.com/docs/app: install on macOS, Windows or Linux, run projects in parallel,
  automations; worktrees/remote/SSH only as nav links (not read). Codex only; ChatGPT login.

### 1.4 Cursor (Cloud Agents, formerly Background Agents)

- Cloud VM agents; worker choice "Cloud, Team Pool, or My Machines"; iOS app (iOS 26+; Android "planned"); push when a turn finishes,
  Live Activities for up to 8 agents; PR review and merge from the phone; **Remote Control** drives a local/SSH workspace from the phone
  (agent loop in the cloud, tool calls on your computer, computer must stay awake). Source:
  https://cursor.com/docs/cloud-agent/web-and-mobile (seen 2026-11-17). A July 2026 secondary summary says self-hosted runtime docs
  were removed: **unverified**.
- Cursor models only for cloud agents; proprietary.

### 1.5 Paseo (closest open-source peer)

- Open-source (Apache-2.0) "agentic development environment": desktop app bundles the daemon; native iOS and Android; web app;
  headless daemon; agents Claude, Codex, OpenCode, Pi, OMP, Antigravity, Muse Code, others through ACP; worktrees optional; remote
  options: optional **E2E-encrypted relay**, direct LAN, your own tunnel (Tailscale/Cloudflare); voice input; 400+ plugins; MCP/CLI/TS
  SDK; "Paseo Hub" for teams; no telemetry. Source: https://paseo.sh (seen 2026-11-17). Star count, activity, security review:
  **unverified**.

### 1.6 Conductor (Melty Labs)

- Mac app; Claude Code, Codex and Cursor; each agent in its own workspace; review and merge; latest release listed 0.94.2; the
  home page says nothing about price, mobile or cloud. Source: https://www.conductor.build/. Search summaries mention Conductor Cloud
  and an iOS app, Apple Silicon only, free: **unverified** (secondary).

### 1.7 Paradev (formerly Superset), `superset-sh/superset`

- MIT, ~16.3k stars; per-task worktree, branch and terminals; diff viewer; in-app browser; PR tools; remote access to another machine
  from desktop, CLI or phone (host must stay awake); iPhone/iPad app **requires Paradev Pro** and iOS 26; about 20 agents; macOS
  (Apple Silicon and Intel), Linux experimental AppImage, **no Windows**; "worktrees do not sandbox processes". Source:
  https://github.com/superset-sh/superset.

### 1.8 Happy (`slopus/happy`)

- MIT, ~24.7k stars; iOS/Android/web/desktop clients, CLI wrapper, `happy-server` relay with a Dockerfile; end-to-end encrypted sync;
  Claude, Codex, Grok, Gemini (Antigravity), OpenCode and ACP agents; invite others to a session. Source:
  https://github.com/slopus/happy. Push-notification behaviour and the security model of the E2E scheme: **unverified** (blog only).

### 1.9 cmux (`manaflow-ai/cmux`)

- macOS-only native terminal on libghostty (Swift/AppKit), vertical tabs, notification rings (OSC 9/99/777), in-app scriptable
  browser, `cmux claude-teams`, session restore (live processes **not** preserved unless opt-in `cmux local-tmux`/`local-zellij`),
  `cmux ssh`, iOS app in TestFlight beta; 28.8k stars; app GPL-3.0-or-later, server parts BSL 1.1. Source:
  https://github.com/manaflow-ai/cmux.

### 1.10 claude-squad (`smtg-ai/claude-squad`)

- Terminal app `cs`: tmux session plus git worktree per task, review/checkout before push, yolo mode (experimental), Claude Code, Codex,
  Gemini, Aider, others via `-p`; needs tmux and `gh`; 8.8k stars; **AGPL-3.0**; no remote machines, no mobile. Source:
  https://github.com/smtg-ai/claude-squad.

### 1.11 Agent of Empires (`njbrake/agent-of-empires`)

- MIT, ~4.6k stars, Rust wrapper on tmux; TUI plus browser dashboard (desktop and phone view with terminal and diffs); Docker/Podman/Apple
  Containers sandboxing; worktrees and multi-repo workspaces; Linux and macOS (Windows via WSL2); eight agents. Source:
  https://github.com/njbrake/agent-of-empires. The closest rival on *sandboxing*; others in the tmux class (`ntm`, Agent Deck, `amux`):
  listed by search only, **unverified**.

### 1.12 Emdash (`generalaction/emdash`, YC W26)

- Apache-2.0, ~6.3k stars; worktree per task; nine agent CLIs; tickets from Linear, GitHub, Jira, GitLab, Asana into agents; PR/CI/merge
  in one place; remote projects over SSH/SFTP with keychain credentials; macOS, Windows, Linux. Source:
  https://github.com/generalaction/emdash.

### 1.13 Nimbalyst (successor of Crystal)

- MIT, ~2.1k stars; visual editors, red/green diff accept/reject per edit, session kanban, worktree per session, Claude Code and Codex
  (OpenCode/Copilot alpha); macOS/Windows/Linux; iOS companion with push (the repo About says iOS and Android; body only iOS);
  sync server `wss://sync.nimbalyst.com`; PostHog analytics (opt-out). Source: https://github.com/nimbalyst/nimbalyst. Crystal is
  deprecated for it (search result and its repo page; not re-opened).

### 1.14 Vibe Kanban (BloopAI)

- Apache-2.0, ~28.7k stars, kanban plus worktree workspaces, diff with inline comments to agents, browser preview, PR flow, ten agents,
  self-host. **"Vibe Kanban is sunsetting"** per its README. Source: https://github.com/BloopAI/vibe-kanban. Lesson: stars did not save it.

### 1.15 OpenHands (now "Agent Canvas")

- MIT, 93.6k stars; self-hosted control centre; runs OpenHands agent or Claude Code, Codex, Gemini, Pi, OpenCode, ACP agents; Docker
  sandboxes (default is the host, with a warning); schedules and webhooks; Slack/GitHub/Linear/Notion; cloud and enterprise
  offerings. Source: https://github.com/All-Hands-AI/OpenHands.

### 1.16 Sculptor (Imbue)

- MIT, research preview, 246 stars; workspace copies, Docker or remote container backend (experimental), Pi and Claude Code. Source:
  https://github.com/imbue-ai/sculptor. Small.

### 1.17 GitHub Copilot coding agent / Agent HQ

- Announced 2025-10-28; Mission Control to assign, steer and track agents across GitHub, VS Code, mobile and CLI; enterprise control
  plane (policies, audit log, allowed-agent lists), metrics dashboard; Codex and Claude added 2026-02-04 (secondary sources); one
  premium request per session. Source: https://github.blog/news-insights/company-news/welcome-home-agents/. Cloud/GitHub-centric; no local machines.

### 1.18 Warp Oz

- Launched 2026-02-10: cloud platform to run "hundreds" of parallel agents in Docker sandboxes, scheduled and API-driven, shareable
  run links and audit trail; self-hosting for enterprises (orchestration still Warp's cloud per a secondary source); May 2026 expanded
  to Claude Code and Codex (secondary). Source:
  https://www.warp.dev/newsroom/2026/2/10/warp-launches-oz-the-orchestration-platform-for-cloud-coding-agents.

### 1.19 Jules (Google) and Devin (Cognition)

- Jules: GitHub-connected VM agent, plan you approve, web, CLI (Jules Tools), REST API, browser notifications. Source:
  https://jules.google/docs. Pricing and parallelism: **unverified**.
- Devin: web app, Slack/Teams, API, "Devin for Terminal" with `/handoff` to cloud, Individual and Teams plans. Source:
  https://docs.devin.ai. Claims such as "75% task completion" are Cognition's own via a third party: **unverified**.
- Both are cloud-first, single-vendor, GitHub-centric; they are not substitutes for supervising your own machines, but they set user
  expectations for "assign from phone, get a PR back".

### 1.20 Not covered or only listed

Zellij-based agent managers (nothing substantial found), MCP multi-agent frameworks and `mux`/Coder, Omnara (the open-source part
appears unmaintained per a comparison blog: **unverified**), AgentsRoom, C3 Mobile, Heeler. Not profiled; mention only as signs that the
phone-control niche is crowded.

## 2. Feature matrix

Legend: **Y** yes, **P** partial, **N** no, **?** unverified or not found on the pages I read. Agora cells cite the proof in this
repo (state on `main` at d3bbaaf0, per `docs/capabilities.md`, which itself says "none recorded here" for most real-machine rows).
Columns: AG Agora; HE herdr; CC Claude Code (Remote Control, agent view, teams); CX Codex app; CU Cursor; PA Paseo; CO Conductor;
PD Paradev; HA Happy; CM cmux; AE Agent of Empires; EM Emdash.

| Capability | AG | HE | CC | CX | CU | PA | CO | PD | HA | CM | AE | EM |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Many machines from one window | **Y** (hub, ADR-0002/0029; LAN run 2026-11-11) | Y (SSH list) | P (via claude.ai, Claude only) | P (SSH, secondary) | P (My Machines/RC) | Y | N? | P | P (relay) | P (`cmux ssh`) | N | P (SSH) |
| Agents survive node/app restart, resume | **Y** (ADR-0060 ledger, ADR-0079 holder, ADR-0075 in-place update) | P (detach yes; server restart kills, resume via integration) | P (supervisor; shutdown stops) | ? | Y (cloud) | P | ? | ? | ? | N (opt-in tmux) | Y (tmux) | ? |
| Git worktree per agent | **P** (optional, ADR-0052; hygiene ADR-0064 not built) | ? | Y | Y | Y (cloud VM) | Y (opt) | Y | Y | ? | N | Y | Y |
| Diff / review UI | **P** (review screen built; base compare, inline comments, merge "not built", `docs/ux/review.md`) | ? | P (Desktop, unverified) | ? | Y (PR review) | Y | Y | Y | ? | N | Y | Y |
| Approvals from the phone | **Y** (Allow/Deny signed by device key, verified on node; `docs/ux/phone.md`, ADR-0020/0028/0061; not tried on a real phone) | P (type into terminal) | Y (RC; push) | ? | P | ? | ? | ? | Y? (blog) | N | ? | ? |
| Mobile / PWA | **P** (PWA, no service worker, ntfy push, ADR-0046/0051) | Y native iOS/Android | Y native | Y (secondary) | Y iOS | Y native | ? | Y iOS (Pro) | Y native | P iOS beta | P web | ? |
| Agent-to-agent messaging | **Y** (structured mesh, board, leases, cross-node, ADR-0008/0031/0039/0057; reply link ADR-0074 not built) | P (CLI/socket prompt, wait, helpers) | P (Claude-only text; teams experimental) | ? | ? | P (MCP) | N | ? | N | P (claude-teams) | N | N |
| Sandboxing / security model | **Y** (OS profiles, secrets held by node, no control socket for agents; ADR-0047/0050/0082-0084; Win off by default; mac 2026-11-14 review) | N? | Y (sandbox settings) | Y (sandbox/approvals pages) | Y (cloud VM) | N? | N? | N ("not a sandbox") | ? | N | Y (Docker/Podman) | ? |
| Cost / usage tracking | **Y** (ADR-0033/0042, per-agent cost in top bar) | ? | P | ? | P | ? | ? | ? | ? | N | ? | ? |
| Notifications | **P** (in-app, desktop shell, ntfy/webhook; no Web Push) | P (in-app; banner) | Y (push) | ? | Y (push, Live Activities) | ? | ? | ? | Y? | Y (rings) | Y | ? |
| Terminal fidelity | **P** (own VT backend ADR-0012; GPU web renderer; TUI client; no head-to-head tests vs Ghostty) | Y (native terminal) | Y | ? | n/a | ? | ? | Y | ? | Y (libghostty) | Y (tmux) | ? |
| Install friction | **P** (signed binary + `agora` front door; no brew/winget/curl one-liner, macOS not notarized; join not built, `README.md`) | **Y** (curl, brew, mise, DMG) | Y | Y | Y | Y | Y | Y | Y | Y (macOS) | Y | Y |
| Open source / licence | **P** (AGPL-3.0 core, Apache SDKs, ADR-0016) | Apache-2.0 | closed | closed | closed | Apache-2.0 | closed? | MIT | MIT | GPL-3 + BSL | MIT | Apache-2.0 |
| Windows / macOS / Linux | **Y** (CI on three, Win real run 2026-11-11; desktop installers unsigned) | Y | Y (Linux desktop beta) | Y | Y | Y | N (Mac) | P (no Windows) | Y | N (Mac) | P (WSL2) | Y |
| Remote / SSH-only machines | **Y** (reverse tunnel + install over SSH, ADR-0032/0034; real SSH run on loopback only) | Y | P | Y (secondary) | Y | Y | ? | Y | Y | Y | N | Y |
| Team / multi-user | **P** (ADR-0049 partly built) | N? | Y (Team/Enterprise) | Y | Y | P (Hub) | N | ? | P (invite) | N | N | ? |
| Agents supported | **P** (L1: claude-code, codex, wrongstack; L2: opencode, wrongstack, antigravity; L3 registry unfinished) | **Y** (23+) | 1 | 1 | 1 | ~7 + ACP | 3 | ~20 | 4+ACP | any | 8 | 9 |
| New agent without a release | **Y** (manifest/ACP/adapter, ADR-0004) | P (needs binary update; local override) | n/a | n/a | n/a | P | N | ? | P | Y (any) | N? | N? |

Honesty notes on Agora cells: "Y" means built on `main` with automated tests; for most rows `docs/capabilities.md` records **no
real-machine evidence yet** (exceptions: LAN run 2026-11-11, macOS 2026-11-14, real SSH 2026-11-09). A competitor "?" is not a "N".

## 3. Where Agora stands

### 3.1 Already ahead (defensible with evidence)

1. **Signed approvals verified on the node** (ADR-0020, 0028, 0041, 0061). No competitor page I read has an equivalent; Claude's
   Trusted Devices gates *viewing*, not node-side verification of each approval. herdr phones type into the terminal.
2. **Per-agent OS confinement plus node-held secrets** (ADR-0047, 0050, 0082-0084); worktrees elsewhere explicitly "do not sandbox".
   Only Agent of Empires (containers) and cloud VMs compete.
3. **Structured cross-vendor, cross-machine mesh** with messages, shared board, file leases and helpers started with a signed OK
   (ADR-0057). Claude's messaging is plain text, Claude-only and goes through Anthropic; herdr is keystroke/CLI level.
4. **Agents outlive the node and updates are in place** (ADR-0075, 0079, 0060). herdr loses processes on server restart; handoff is
   experimental. cmux does not preserve live processes by default.
5. **No account, no third-party relay in the path**; hub is yours; hub cannot widen node policy; secrets never leave the node.
6. **Usage/cost from the agents' own files** (ADR-0042) across vendors in one view (competitors mostly per-vendor).
7. **Localization from day one** (en, tr; ADR-0007). No competitor page mentioned it.

### 3.2 Behind or missing, ranked by (how many competitors have it) x (user value)

| # | Gap | Competitors that have it | User value |
| --- | --- | --- | --- |
| 1 | Agent coverage: 3 terminal manifests vs 20+ | herdr (23+), Paradev (~20), Emdash (9), AoE (8), Paseo | Very high: users run what they already own |
| 2 | One-line install, notarized/signed installers, join/pairing (ADR-0065 not built) | herdr, Paseo, Conductor, Emdash, all cloud tools | Very high: time to first agent |
| 3 | Review to PR loop: base compare, inline comments to agent, merge/PR, CI checks | Conductor, Emdash, Paradev, Nimbalyst, vibe-kanban, Cursor | High |
| 4 | Native phone app with real push (Live Activity, lock screen) | herdr, Paseo, Happy, Cursor, Claude, Codex, Nimbalyst, Paradev | High |
| 5 | Remote reach rungs 3 and 4 (Tailscale, Cloudflare) not built; direct data path not built (ADR-0023) | Paseo, Happy (relay), herdr (SSH+Tailscale discovery) | High |
| 6 | Worktree-per-agent as the easy default and hygiene (ADR-0052 made it optional; ADR-0064 not built) | claude-squad, Conductor, Claude, Codex, Emdash, AoE | Medium-high |
| 7 | Task intake and automation: issues (Linear/GitHub/Jira), schedules, webhooks | Emdash, OpenHands, Warp Oz, Claude routines, Codex automations, Devin, Jules | Medium-high |
| 8 | Ephemeral/cloud machines (spin up a sandbox node) | Cursor, Claude cloud, Oz, Jules, Devin, OpenHands Cloud | Medium |
| 9 | Plugins/mods, scripting API for agents and users | herdr, Paseo (400+), cmux | Medium |
| 10 | Team/enterprise (SSO, org policy, audit UI) | Agent HQ, Claude Team/Enterprise, Cursor, Oz | Medium (later) |
| 11 | Licence: AGPL core vs Apache/MIT peers | herdr, Paseo, Emdash, Paradev, Happy | Adoption risk, a maintainer decision (ADR-0016) |
| 12 | Voice input | Paseo, herdr mobile, Cursor | Low-medium |

## 4. The ten things to build next

Effort S (days), M (1-3 weeks), L (a month+). "ADR" = needs one under AGENTS.md (invariant, protocol/schema or dependency policy).

1. **Agent coverage sprint (L1 manifests for the top ~15 CLIs) plus remote manifest updates.** Cursor Agent CLI, Gemini CLI, Copilot CLI,
   OpenCode (terminal), Amp, Droid, Kimi, Qwen, Cline, Kiro, Grok, Pi, Antigravity CLI, Devin CLI. herdr's whole moat is its detection
   list; Agora's "agents are data" invariant makes this cheap, and fixture screens are already the test method. Also ship signed
   manifest updates (ADR-0018 exists) so a new agent is a download, not a release, which turns herdr's "needs a binary update"
   weakness into an Agora advantage. Effort M (S per manifest). ADR: no for manifests; yes only if remote manifest fetch changes the
   trust model beyond ADR-0018.
2. **A front door that wins the first five minutes.** `curl | sh`, Homebrew, winget/scoop, `.deb`/`.rpm`, a notarized macOS build and a
   signed Windows binary (ADR-0077), plus the join string and pairing (ADR-0065, `docs/specs/pairing.md`). herdr installs with one
   line on all three systems; Agora's README still starts from "download three files and verify a signature". Keep the signature
   check, make it automatic. Effort M-L. ADR: no (ADR-0062/0065/0077 accepted); the installer-script trust model deserves a note.
3. **Close the review-to-PR loop.** Base-branch compare, inline comments sent to the agent, "create PR", CI checks, merge/rebase/discard
   for managed worktrees (all listed "not built" in `docs/ux/review.md`), and the `files.read at a revision` message that review
   already needs. Every GUI competitor has it; it is what makes people leave the terminal. Effort L. ADR: yes (new protocol messages;
   schema changes need one).
4. **Approvals and attention on a real phone: push that works with the app closed.** ADR-0051 chose ntfy over Web Push; the cost is
   that iOS instant delivery needs an upstream and the PWA has no service worker. Options: revisit Web Push with a stated privacy
   cost (ADR-0051 item 4 says to), or a thin Tauri/Capacitor mobile shell with APNs/FCM. Keep the signing path identical (the
   differentiator: Allow/Deny signed on the phone, verified on the node). Test on a real iPhone and Android and record it. Effort L.
   ADR: yes (supersedes parts of 0046/0051; adds a hub-held key or a native dependency).
5. **Remote access rungs 3 and 4, then an optional end-to-end-encrypted relay.** `agora remote tailscale` and `agora remote cloudflare`
   (ADR-0067, not built), plus direct client-to-node data path (ADR-0023). Without them "from a phone anywhere" means "set up a VPN
   yourself". An opt-in E2E relay (as Paseo and Happy offer) would cover NATed laptops without Tailscale; the relay must see only
   ciphertext and must not weaken "hub cannot widen policy". Effort M (rungs), L (relay). ADR: rungs no (accepted); relay yes.
6. **Make isolation the easy, visible default.** When a second agent starts in a repo that already has one, offer "own worktree" by
   default and show collisions (file leases already exist in the mesh; surface them in the UI). Build workspace hygiene (ADR-0064) so
   worktrees do not eat the disk. ADR-0052 keeps worktrees optional and that is right for the product; a default prompt is a UX change,
   not an invariant change. Effort S-M. ADR: no, unless ADR-0052's text is amended.
7. **Task intake and automations.** Start an agent from a GitHub/Linear issue, a schedule, or a webhook; run unattended with
   pre-approved, scoped signed grants (ADR-0041 already models grants). This is what Emdash, Oz, Claude routines and Codex automations
   sell, and unattended runs with a node-verified grant is a place Agora can be safer than all of them. Effort M-L. ADR: yes
   (unattended starts touch approval authority and add protocol messages).
8. **Ephemeral nodes: "start a sandbox machine" in one click.** A node image (Docker/Firecracker/any cloud VM or a Hetzner-style
   provider script) that enrolls itself, runs one task and disappears, using the existing SSH install path (ADR-0034) and confinement.
   Answers Cursor/Claude cloud/Oz/Jules without moving secrets (secrets stay on the user's own nodes). Effort L. ADR: yes (new
   lifecycle and secret-handling rules).
9. **Evidence and a public benchmark against herdr and Paseo.** `docs/capabilities.md` shows "none recorded here" for most rows.
   Run each on real machines (including a real phone), record the dated notes, and publish a reproducible comparison: keystroke
   latency, time to first agent, reconnect time, what survives a node restart, approval round-trip. Claims in `docs/vision.md`
   ("multiplexers: partial multi-machine, inferred state") should be rewritten from this data. Effort M. ADR: no.
10. **Scriptable control surface and plugins.** herdr (CLI+socket, plugins), cmux, Paseo (CLI, MCP, TS SDK, plugins) make the tool
    extensible by users and by their agents. Agora has the hub protocol and the mesh, but no documented, stable "automation API for
    people" (`agora agent list --json`, wait-until-blocked, events subscription) or plugin point (panel registry is "not built" in
    ADR-0068). Ship a small, documented CLI/JSON surface first, plugins later. Effort M. ADR: CLI surface no; plugin runtime yes.

Deliberately not on the list: building Agora's own agent (non-goal), cloud-hosted inference, voice input (low value per effort), and
a second terminal-emulator effort before benchmarks exist.

## 5. Risks and open questions for the maintainer

- **Licence (ADR-0016).** AGPL-3.0 core is the outlier among open peers (herdr, Paseo, Emdash Apache-2.0; Paradev, Happy, Nimbalyst,
  OpenHands MIT; claude-squad AGPL is the other exception). It protects a future hosted offering but slows adoption by companies and
  by people who wrap Agora in their own tools. Revisit consciously; it is not a technical fix.
- **Pace.** herdr ships several releases per week; Agora's 0.3.15 is in "pre-alpha" per its README. The ladder in ADR-0062 is right, but
  rungs 2 and 5 of it (join, pairing) are exactly what herdr does well.
- **Do not claim "first" or "only" for multi-machine or mobile.** Say "approvals signed by your device and checked on the machine,
  agents confined by the OS, structured mesh across vendors and machines".
- **Vibe Kanban's sunset and Crystal's deprecation** show that free, local-only worktree wrappers do not sustain themselves; a
  hub-and-team story (ADR-0049) is where durable value is, but only after rungs 1-4 above.

## 6. Method and limits

- Searches: "herdr", mobile/relay, Conductor, Nimbalyst/Crystal, Agent HQ, Codex app, Cursor background/cloud agents, Happy/Omnara,
  Warp Oz, Jules/Devin, tmux/zellij agent managers. Fetched each project's own repo or docs where possible.
- Not verified at all: Codex official announcement (HTTP 403), Zellij agent managers, `ntm`/Agent Deck/`amux`, Omnara, AgentsRoom,
  Conductor cloud/iOS, Cursor self-hosted status, any pricing, any performance number of a competitor.
- Fetch tool summaries were produced by a small model, so each quoted figure (stars, versions) is "as the page showed" and should be
  rechecked before it is used in public material.
