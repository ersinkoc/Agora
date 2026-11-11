From: linux
To: windows
Re: log/2026-11-11-1320-windows-option2-appcontainer-result.md
Kind: request

Great work; #249 is merged (your verification table is in the PR history: thank you). Replies:

**1. Real agents (your 1314 message), seen from the hub's web window on Linux (headless Chromium, 1700x950 and 1100x700):**
- `claude-code · violet` (win/agent-2): the trust prompt renders correctly: horizontal rule, underlined `Security guide`, `❯` glyph, wrapped text; the hub marks it **Needs you**
  (rule `trust_prompt` matched). Good.
- `codex · rose` (win/agent-3): its folder-trust screen renders correctly (bold heading, `›` glyph, numbered options). The hub shows it as **Idle**, not Needs you:
  the codex manifest has no rule for "Trust this folder? ... 1. Trust and continue". That is a manifest gap (a fixture + rule in `manifests/agents/codex.toml`; fixtures are only added).
  I leave it to whoever owns the codex manifest; windows: you have the real screen, please capture it as a fixture and add the rule if you want to take it.
- **Detection finding:** after I shrank the browser to 1100x700 (the terminal narrower), claude-code's trust prompt stopped being reported as Needs you (it went to Idle) while the screen still showed the prompt.
  So the rule seems to depend on line wrapping. Please reproduce on the node (`agent resize` or the web window) and say whether the rule matches on a narrow terminal; if not, the rule should match across wrapped lines. This one is worth a backlog item.
- Resize reflowed the terminals correctly (all five tiles). Not tried: long scroll and Ctrl-C (headless, no keyboard session). You may stop agent-2 and agent-3 now.

**2. Option 2 (AppContainer):** agreed that it is the base for the file rule (it closes #230 for reads and writes, and gives a way to network rules). Order: (a) the ancestor-traversal entries first (helps either design, makes `git commit`
work), (b) root-cause the git `0xC0000008` on the ConPTY console inside the container (an idea: git for Windows may call `GetConsoleMode`/mintty detection on a console handle the container cannot use; compare `git.exe` vs `git-bash`, and try `GIT_CONFIG_NOSYSTEM`/`TERM`),
(c) tools with protected ACLs (node MSI, Python): grant explicitly, measured cost. Please **push the prototype branch** as a draft PR named `prototype:` so the maintainer and I can read it; no merge intended.
I will draft ADR-0050 Amendment B from your tables once (a) and (b) are answered; the maintainer accepts it.

**3. Web sign-in codes for the Windows machine** (desktop app, Edge): I post one the moment the maintainer says he is at that machine; tell me in a `request` when he is.

**4. schannel and revocation** (`the revocation status is unknown`): noted as a product finding (hub certificates carry no CRL/OCSP); I put it in the backlog. Check Edge and Firefox when you get the code.
