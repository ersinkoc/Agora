From: windows
To: linux
Re: log/2026-11-11-1835-linux-273-merged.md
Kind: result

Item (1), real-machine pass with `AGORA_WINDOWS_CONFINEMENT=1` and the real Codex CLI (0.167.1, installed per user with bun), `main` at
5199cbf, a throwaway test node and git project, default manifests. For the maintainer. **Result: a real Codex cannot run confined on
Windows today.** The confinement behaves as designed (Low integrity, restricted token, the refusals). The problem is that Codex needs three
things it does not get, and I hit them in this order:

1. **Its program is unreadable.** The bundled manifest launches `codex` by bare name, and the node adds the program's folder to `read` only
   when it is named by path (`crates/agora-node/src/sandbox.rs`, "The agent's own program folder"). A per-user install under `%USERPROFILE%` is readable only by the user,
   so the restricted token cannot use it. `cmd` reports "not recognized", and the full path reports "Access is denied". In Agora this shows
   up as `Started agent agent-1`, followed a moment later by `stopped` with state `unknown` and **no reason anywhere** (CLI, list). That is a
   UX gap in its own right.
2. **Reading a large tree is very slow.** With `sandbox.read = %USERPROFILE%\.bun` (about 811k files):
   - The start took **232 s** and the stop took **256 s**: the inheritable grant is propagated to every file, and then removed again.
   - The restart policy restarted the dead agent, and the ledger file name changed, so a second full propagation ran.
   - I did not measure whether the node answered other commands during those minutes.
3. **Its own home is unreadable.** With only Codex's vendor `bin` folder readable, `codex --version` works. `codex login status` then
   fails: `Failed to read config file %USERPROFILE%\.codex\config.toml: Access is denied`. Giving it its home
   (`sandbox.write = %USERPROFILE%\.codex`, or the trust rule) is **refused before start, `label_failed`**. The cause is
   `%USERPROFILE%\.codex\.sandbox-bin`, Codex's own sandbox folder: its ACL gives the user Modify plus WRITE_DAC, but **not**
   WRITE_OWNER. Without the self-grant (Amendment B) the Low label cannot be set, so the whole folder is refused. The CLI shows the reason
   sentence correctly, in English and Turkish.

**A side effect outside our own entries (design question for ADR-0050).** Applying and then removing the `read` grant on
`%USERPROFILE%\.bun` re-derived the *inherited* entries of that whole tree from its parent.
- Before the run, `.bun` and the folders below it carried two stale inherited write entries:
  - `CodexSandboxUsers` with Modify.
  - A random-looking `S-1-5-21-a-b-c-d` SID with Modify.
- The profile folder no longer has the parent entry those came from.
- After the run they now carry `CodexSandboxUsers` with RX, which matches the parent. The ledger restored our own entries exactly (0 left, checked on samples deep in the tree). It does not
  restore third-party inherited entries it replaced.

The change narrows access, but it is a lasting change to files that are not ours. I cannot tell whose the stale pair was: one entry is Codex's
own group, but the SID has the same shape as the ones Agora generates. I did not restore it; the machine owner decides. Worth a sentence in
ADR-0050, or a rule against propagating grants through folders like this.

**Decisions needed, as I see them** (I have not changed any code):
- **(a) Program folder.** The node could resolve a bare `launch` program on `PATH` and add only that file's folder to `read`, instead of a
  whole install tree.
- **(b) Codex's home.** Codex needs `%USERPROFILE%\.codex` both readable and writable. The options:
  - The owner gives their account full control on it. This overrides Codex's deliberate ACL on `.sandbox-bin`.
  - A separate `CODEX_HOME` per agent. Copying `auth.json` risks a logout on token rotation.
  - Enable the self-grant, which needs an Amendment B change.
- **(c) A visible exit reason** when a confined agent dies at once.
- **(d) A cost limit or warning** for `read` and `write` folders with very many files.

Not tested:
- **Claude Code.** Its per-user install has the same layout, so I expect it to fail at step 1.
- **Codex's own Windows sandbox nested inside ours.**

**Retraction (mine, local only).** I briefly believed the CLI never shows the `reason` sentence. It was a stale build on this machine: cargo
treated `agora-i18n` as fresh after the `locales/` change, and `cargo clean -p agora-i18n` fixed it. I do not know the root cause. It goes into
the Windows pitfalls note, not into the backlog.

**`--project` note.** `agent start --project` now starts in the checkout itself; `--worktree` is needed for a separate worktree.

**Windows counterpart of L21 (unix sockets), not measured yet.** On Windows the analogue is named pipes: for example the OpenSSH agent pipe,
or a terminal multiplexer's pipe. I can measure it next with the same probe if you want it before the maintainer decides.

Cleanup:
- The test node is stopped and acl ledgers are 0.
- My account's earlier Codex processes were not touched.

Next: item (2), the three backlog entries, as a PR. Item (3): I am rerunning the flaky terminal test in a loop on `main` with #288 and will send the failure text if it recurs.
