# 0105. Confined Node.js agents on Windows: keep the restricted token, fix the stdio pipe in libuv
Status: Accepted
Date: 2026-11-23

Accepted: 2026-11-23 (the maintainer, in the merger chat, to the question summarising the recommendation: "Accept, we file upstream". The
project keeps ADR-0050 as it is and files the minimal libuv/Node report itself; #895 stays open until a fixed Node is verified).

Origin: issue #895 (`needs-adr`). The maintainer chose on 2026-11-23 to draft a Proposed ADR, written by the Windows session because
it needs measurements on a Windows machine. Roadmap M2.3.

Touches ADR-0050 (Windows agent confinement, Accepted, Amendment B), ADR-0047 (confinement and its `worktree` level), ADR-0045 (the
job object), #897 (adapter and ACP agents refused at start on Windows; PR #1024), #898 (the documented limits) and
`docs/security/sandbox-recipes.md`. Code references are at main `96796b15`.

## Context

### What fails

A confined Node.js program on Windows cannot start a child with piped stdio: `child_process.spawn` and `spawnSync` with
`stdio: 'pipe'` (Node's default) fail with `EPERM`. With `stdio: 'ignore'` or `'inherit'` they work. So a Node-based agent (Claude
Code, Gemini CLI and others) cannot run a tool or an MCP server while confined on Windows. `docs/security/sandbox-recipes.md` says
so today.

The cause is one access check (measured 2026-11-22 and again below):
- Node 26.13.0 bundles libuv 1.52.1. In libuv's `src/win/pipe.c` at tag v1.52.1, `uv__pipe_server` creates the server end with
  `CreateNamedPipeA(name, access | FILE_FLAG_FIRST_PIPE_INSTANCE, …, NULL)`. The name is `\\?\pipe\uv\%llu-%lu`, made by
  `uv__unique_pipe_name`, and the security attributes are NULL. `uv__create_pipe_pair` then opens the child's end by name with
  `CreateFileA`, with `sa.lpSecurityDescriptor = NULL`. For a write end it asks for `GENERIC_WRITE`, plus `FILE_READ_ATTRIBUTES`
  (or `GENERIC_READ`), plus `WRITE_DAC`.
- A named pipe created without a security descriptor gets the pipe file system's fixed default: full control for SYSTEM,
  Administrators and the owner, and read for Everyone and Anonymous. It does not get the token's default DACL, which ADR-0050 sets
  so that it names the agent's SID (`set_default_dacl` in `crates/agora-platform/src/winconfine.rs:1292`, used at `:1425`).
- A restricted token passes only when the normal check and the check against its restricting SIDs both pass. Its restricting SIDs
  are the agent's SID, Everyone, Users and the logon SID. Of those, the default descriptor names only Everyone, and only for
  reading, so opening the pipe for writing is refused (error 5), and libuv reports `EPERM`.
- The owner entry would be enough, but a restricted token's owner cannot be the agent's SID or the logon SID: `SetTokenInformation`
  answers error 1307 (measured 2026-11-22).

### How it was measured

Measured on WHITE (Windows 11 Pro 10.0.26300) on 2026-11-23, with Node v26.13.0, Git for Windows, and Python 3.15.2 under `C:\Python315`.

A throwaway harness, not part of the repository, did the following for each option:
- It built the token the same way `restricted_token` does (`winconfine.rs:1333`), or the variant named below.
- It started `cmd` in a pseudo-console, as the node starts a terminal agent, and ran a fixed battery of checks.

The folders and files:
- **Agent A's folder:** the working folder, with A's SID on its ACL and a Low label.
- **Agent B's folder:** a second agent's folder, prepared the same way for B.
- **Outside folders:** one on the data volume, one under the user profile, and `C:\ProgramData`.
- **A user-private file** under `%TEMP%`.

All lab folders were on a data volume whose root grants `Users` read. The battery:
- Node `spawnSync` and `spawn` with pipes.
- A stand-in for libuv's pipe pair with the same call shape: server `PIPE_ACCESS_INBOUND | WRITE_DAC | FILE_FLAG_FIRST_PIPE_INSTANCE`,
  client opened by name for `GENERIC_WRITE | FILE_READ_ATTRIBUTES | WRITE_DAC`. It runs once with the default descriptor and once with
  an explicit descriptor that carries the token's default DACL.
- A Rust child with piped stdio, `cmd`, `git init` and `git commit`, Git's `sh`, and `python`.
- The writes, plus the reads of the user-private file and of B's folder.

Each option, as built:
- **(a) AppContainer:** no capabilities. A's package SID is on A's folder, which has a Low label, and B's package SID is on B's folder.
- **(b) No restricting SIDs:** the user's own token at Low integrity, with a Low label on both agents' folders. It was run three
  times: with no deny entry; with a write-deny entry for the Low integrity SID (`S-1-16-4096`) on B's folder; and with a write-deny
  entry for the logon SID on B's folder.
- **(c) A wider restricting list:** the ADR-0050 token plus SYSTEM, or plus Administrators. These are the SIDs the default pipe
  descriptor grants full control.
- **(c′) Write-restricted:** the ADR-0050 token created with `WRITE_RESTRICTED`.
- **(d) The ADR-0050 token unchanged**, with only the pipe made differently (the stand-in's explicit descriptor).

### Results

| Check | ADR-0050 today | (a) AppContainer | (b) Low, no restricting SIDs | (c) + SYSTEM | (c) + Administrators | (c′) write-restricted |
|---|---|---|---|---|---|---|
| Node `spawnSync`, pipes | EPERM | Node does not start (below) | ok | ok | ok | EPERM |
| Node `spawn`, pipes | EPERM | Node does not start | ok | ok | ok | EPERM |
| Pipe pair, default descriptor | refused (client, 5) | refused (server, 5) | ok | ok | ok | refused (client, 5) |
| Pipe pair, explicit default-DACL descriptor: **(d)** | **ok** | refused (server, 5) | ok | ok | ok | ok |
| Pipe pair under `\\.\pipe\LOCAL\` | refused (client, 5) | ok | not run | not run | not run | refused (client, 5) |
| Rust child with piped stdio; `cmd` child | ok | ok | ok | ok | ok | ok |
| `git init` and `git commit` in A's folder | ok | fails: cannot `stat` the folders above | ok | ok | ok | ok |
| `python --version` | ok | not found | ok | ok | ok | ok |
| Git's `sh` | fails, 0xC0000142 | fails, 0xC0000142 | fails, 0xC0000142 | fails, 0xC0000142 | fails, 0xC0000142 | fails, 0xC0000142 |
| Write in A's folder | ok | ok | ok | ok | ok | ok |
| Write in **B's folder** | refused | refused | **written** | **written** | **written** | refused |
| Write outside (data volume, profile, `C:\ProgramData`) | refused | refused | refused | refused | refused | refused |
| Read the **user-private file** | refused | refused | **read** | **read** | **read** | **read** |
| Read a file in B's folder | read (`Users` read inherited) | refused | read | read | read | read |

Option (b) with deny entries on B's folder:
- **Deny for the Low integrity SID:** no effect. A still wrote in B's folder, so DACL entries do not match integrity SIDs.
- **Deny for the logon SID:** A was refused in B's folder. So was the user's own unconfined process (measured), and so would agent
  B be, since every process of the session holds that SID.
- **Everything else** was unchanged from the (b) column, including the user-private file being read.

Notes on the table:
- **Node in the AppContainer.** The installed `node.exe` cannot be started, because `C:\Program Files\nodejs` grants `Users`
  but not `ALL APPLICATION PACKAGES`. The same holds for `C:\Python315`. A copy of `node.exe` placed in A's folder exits before any
  user code: its module loader's `realpathSync` gets `EPERM` on `lstat 'D:\'`. Inside the container, even `cmd`'s `dir` of A's own
  folder by full path is refused, because it enumerates the folders above.
- **Named pipes in the AppContainer.** The container may create pipes only under `\\.\pipe\LOCAL\`. libuv's `\\?\pipe\uv\…`
  pipes would be refused even if every grant above were made. Read from the code, not run: `uv__pipe_server` treats
  `ERROR_ACCESS_DENIED` as a name collision and retries with the next name, so in a container it would loop instead of failing.
- **Git's `sh`.** It fails in every Low-integrity variant. msys reports `NtCreateDirectoryObject(\BaseNamedObjects\msys-2.0…)`
  refused (0xC0000022). This is the Git Bash limit already documented (#898). It comes from the Low integrity level, not from the
  restricting SIDs, and no option here changes it.
- **Not reached.** The pseudo-console failure of git that ADR-0050 Amendment B recorded for AppContainer (0xC0000008) was not
  reached here, because git stopped earlier.

## Decision

1. **ADR-0050 Amendment B stays as it is.** None of (a), (b) or (c) is adopted for #895:
   - **(a)** does not make Node's piped spawn work: libuv's pipes are refused outside `\\.\pipe\LOCAL\`. It also breaks Node,
     Python, git and `sh` unless an administrator grants `ALL APPLICATION PACKAGES` on the drive roots, the folders above every
     worktree, and every toolchain folder. Amendment B's other reasons stand.
   - **(b)** makes Node work but gives up the two guarantees the maintainer accepted in ADR-0050's revised decision: a second
     agent's folder is written, and user-private files are read. A deny entry cannot restore them. It would need a SID that is in
     agent A's token and in no other process's token, and a token derived from the user's own has none. Both candidates were
     measured: a deny entry for the Low integrity SID has no effect, and one for the logon SID also locks out the user and agent B.
   - **(c)** makes Node work for the same reason it breaks the rules. SYSTEM and Administrators sit on almost every folder's ACL,
     so B's folder becomes writable and private files readable, the same loss as (b). The write-restricted token (c′) does not
     help Node and opens reads.
2. **The fix belongs in libuv (d).** The stdio pipe pair should be created with an explicit security descriptor that carries the
   creating token's default DACL, instead of the pipe file system's default. Under the unchanged ADR-0050 token, the stand-in passes
   with that one change. The project reports this upstream with a minimal reproduction that does not need Agora: a restricted token,
   a piped `spawnSync`, and `EPERM`. The maintainer decides who files the report. Agora's code does not change. No existing libuv or
   Node.js report of this was found on 2026-11-23. The nearest is nodejs/node#47086, which notes that libuv creates its pipes with
   `WRITE_DAC` and default security; it is about changing pipe permissions, not about restricted tokens.
3. **Until a Node release carries the fix, the behaviour and the documentation stay as they are.**
   - `docs/security/sandbox-recipes.md` keeps saying that a confined Node-based agent cannot start piped children on Windows.
   - `docs/capabilities.md` keeps #895 open.
   - Owners who need Node-based agents on Windows run them with `sandbox = off`. The sandbox level is set per node today (the
     per-project and per-kind choice of ADR-0047 decision 1 is not built), so this leaves every agent on that node unconfined.
4. **When a fixed Node is released**, the Windows session re-runs these measurements with a confined Claude Code that runs a tool
   and an MCP server. It then updates `docs/capabilities.md` and the recipes, and closes #895.

## Consequences

- No code, schema or test changes in Agora. The confinement token and its tests stay as they are, so the existing confinement tests
  cost nothing. Options (a), (b) and (c) were measured in the throwaway harness only and not prototyped in `agora-platform`, so their
  cost to the existing confinement tests was not measured.
- Node-based agents stay unusable confined on Windows until the libuv or Node fix ships, and that timing is outside the project.
  Rust agents (Codex) and the shell kind keep working confined.
- #897 (confining adapter and ACP agents on Windows) is not unblocked. Most adapters are Node programs: they would need this libuv
  fix and a platform `spawn_confined` for non-PTY processes, and the latter is still unbuilt.
- Git's `sh` stays unsupported confined on Windows whatever is decided here (measured above).

## Alternatives considered

- **(a) AppContainer, (b) Low integrity without restricting SIDs, (c) a wider restricting list, (c′) a write-restricted token:**
  measured above and not adopted.
- **A separate local Windows account per agent, or one for all agents:** not measured. It needs an administrator to create the
  accounts, a credential the node would hold (secrets never leave the node, but this adds one), and a profile per account. It
  conflicts with a no-administrator install.
- **Patching or injecting into Node** (for example, replacing its pipe creation at run time): rejected. It modifies a third-party
  program, breaks with each Node release, and would ship code into every confined agent.
- **A separately named, weaker Windows policy built on (b):** possible under ADR-0050's own review ("a separately named, explicitly
  weaker policy with its own consent and threat-model treatment"). It is not proposed here. Whether it is worth having is the
  maintainer's call, and it would need its own ADR.

## Open questions for the maintainer

1. Who files the upstream report (libuv, or Node.js), and should the project offer the patch as well as the reproduction?
2. Should a start under `sandbox = worktree` on Windows warn when the agent is Node-based? The node cannot tell today. It would need
   a new manifest field such as `runtime = "node"`, which is a separate decision.
