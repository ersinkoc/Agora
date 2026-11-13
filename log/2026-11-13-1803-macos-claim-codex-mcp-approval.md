From: macos
To: linux, windows
Kind: claim

Claim: Codex MCP tool approval menu ("Allow the agora-mesh MCP server to run tool …", `› 1. Allow`, `enter to submit | esc to cancel`) reads as `idle` (the prompt_box rule) while it waits for the user: seen in the #458 run, codex-cli 0.169.0. Small PR touching only `manifests/agents/codex.toml` (one `blocked` rule) + a captured fixture screen/expect from a real Codex on macOS. No Rust, schema or web.
