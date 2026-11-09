# 0042. Token usage read from the files an agent already writes
Status: Accepted: 2026-11-09 (maintainer's explicit approval in chat)
Date: 2026-11-09

## Context
Cost and quota views need tokens per agent (ADR-0021). The manifest format reserves `[usage] source = "otel" | "file" | "adapter" | "none"`
(`docs/specs/agent-manifest.md`), but the engine ignores the section, so an L1 agent (Claude Code, Codex, WrongStack) reports no usage and
the web cost view stays empty for it. L2 and L3 agents report usage through the adapter protocol; only terminal agents have the gap.

Both agents we ship already write what we need, in different shapes (checked on this maintainer's machine, 2026-11-09):
- **Claude Code** appends one JSON line per message to `~/.claude/projects/<cwd with / as ->/<session id>.jsonl`. An assistant line has
  `sessionId`, `timestamp`, `cwd`, `message.model` and `message.usage.{input_tokens,output_tokens,cache_read_input_tokens,
  cache_creation_input_tokens}`. Each line is that message's own usage (a delta); a message can appear on more than one line.
- **Codex** appends to `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<session id>.jsonl`. An `event_msg` line of type `token_count` carries
  `info.total_token_usage.{input_tokens,cached_input_tokens,output_tokens,...}`: a running total for the session, not a delta.

## Decision
1. **A manifest describes the file; the node reads it. No agent-specific code.** `source = "file"` takes a `[usage.file]` table:
   `glob` (with `{home}` and `{config_dir}` placeholders, as for `[mcp]`), `format = "jsonl"` (only this for now), `select` (a field and
   value that a line must have, for example `type = "assistant"`), the JSON paths `model`, `input`, `output`, optional `cache_read`,
   `cache_write` and `session`, and `mode = "delta" | "total"`. For `delta` the node adds each line; for `total` it reports the growth
   since the last value it saw for that file. A line that does not parse or lacks the paths is skipped, never guessed at.
2. **Which file is the agent's.** The node only reads files whose `session` path value equals the agent's native session id when it knows
   it (hook or screen-reported), otherwise files created after the agent started whose `cwd` path value equals the agent's worktree. A
   file that matches neither is ignored, so another session of the same tool is never counted for this agent.
3. **Reporting.** The node emits the existing `AgentUsageReported { agent_id, model, input_tokens, output_tokens }` (`cache_read` and `cache_write`, when the manifest
   names them, are added into `input_tokens`: all prompt-side tokens the model processed); no protocol change.
4. **Limits.** The node reads from a saved offset, at most 1 MiB per second per agent, ignores lines over 256 KiB, follows at most 4 files
   per agent, and stops when the agent exits. Reading is local file access with the user's own permissions; nothing leaves the node but the
   four numbers above. `{home}` expands only to the node user's home directory.
5. **Hooks are not in this ADR.** `[hooks]` (installing a reporter into the agent's own settings) changes files the agent trusts and
   deserves its own decision; session identity falls back to the rule in point 2 until then.
6. Shipped manifests add `[usage]` for `claude-code` (`mode = "delta"`, de-duplicated by `message.id`) and `codex` (`mode = "total"`),
   each with a fixture file of real lines (scrubbed) and a test.

## Security
- A usage file is untrusted data: numbers are bounded (saturating add), strings are not shown, paths are never taken from file content.
- The glob is the manifest's, the hub cannot change a manifest on a node (L1 manifests live on the node), so a hub cannot make a node read
  arbitrary files.
- A malicious agent could write a misleading file; the effect is a wrong cost figure for that agent, nothing else.

## Consequences
- Engine: a `UsageFile` section in `agora-adapters::manifest`, a reader in `agora-node` that runs beside the screen detector, fixtures and
  tests (offset resume, total vs delta, duplicate message ids, foreign sessions ignored, oversized and malformed lines).
- Docs: `agent-manifest.md` `[usage]`, `docs/design/resource-limits.md`.
- No schema change, so no new golden fixtures.

## Alternatives considered
- **OpenTelemetry (`source = "otel"`)**: Claude Code and Codex can export it, but it needs a local collector port per agent and the user's
  telemetry configuration; files are already there.
- **Per-agent Rust parsers**: break "agents are data".
- **Parse the screen's token counter**: fragile, and not every agent shows one.
