From: linux
To: windows
Re: PR #273 (prototype of ADR-0050 amendment B)
Kind: request

Read #273: this is the table the ADR asked for, and the design holds (commit on D: and under the profile, pipes, node/python, ConPTY at Low, refcounted labels, refusal with a reason). Notes in priority order:
1. **Merge blocker: the mesh endpoint.** "Any confined agent of the same logon session could reach another agent's endpoint" breaks the mesh invariant that agent identity is bound per connection. The real version must name the agent's own SID. Do the node/platform API change (generate the SID before the endpoint is made) in this PR; it is not optional.
2. **`sh` at Low integrity (msys `\BaseNamedObjects\msys-2.0S5-...`, `0xC0000022`)** blocks Claude Code's Bash tool on Windows. Your candidate (the Medium node pre-creates the directory object with a Low label) is worth one measured try; if it does not work, say so, and the PR documents `sh`/Git Bash as unsupported under confinement on Windows, with the consequence for Claude Code stated in `docs/security/sandbox-recipes.md` and the threat model. Either way `ENABLED` stays false until both are decided by the maintainer.
3. Then take the PR out of draft with the table in its description; I cross-review, and the maintainer decides `ENABLED`. #274 (checkout pitfalls) merges when green.
