# For developers on several machines

The maintainer works on Agora from three machines (Linux, macOS, Windows) with a coding agent on each. For now the
**repository is the only link between them**. This page says how they talk through it. Each machine has its own page:

- [for_linux_dev.md](for_linux_dev.md)
- [for_macos_dev.md](for_macos_dev.md)
- [for_windows_dev.md](for_windows_dev.md)

Everything in `AGENTS.md` still applies (one task, one branch, DCO sign-off, Conventional Commits, no `unwrap`, platform code
only in `agora-platform`, text only in `locales/`). This page adds only the cross-machine part.

## Two channels

| What | Where | How |
| --- | --- | --- |
| **Code and docs** | pull requests to `main` | one branch per task: `<agent>/<machine>-<slug>`, for example `claude/windows-confinement-check` |
| **Messages and results** | the `comms` branch, folder `log/` | a new file per message, pushed straight to `comms` (no pull request, no CI) |

The `comms` branch exists so that "I ran it on Windows, here is what happened" never waits for a review and never conflicts with code.
It holds only `log/*.md` files and is never merged into `main`.

## Sending a message

Create a **new** file; never edit or delete a file someone else wrote. Name it
`log/YYYY-MM-DD-HHMM-<from>-<slug>.md` (UTC time; `<from>` is `linux`, `macos`, `windows` or `maintainer`), with this header:

```markdown
From: windows
To: linux            (a machine, or "all")
Re: log/2026-11-11-1130-linux-enable-windows-confinement.md    (optional: the message you answer)
Kind: result | question | request | blocked | note
Branch: claude/windows-confinement-check    (optional)

Body. Say what you ran (exact commands), what you saw (paste the output that matters, not all of it), and what you need from the
reader. One topic per message.
```

Push it:

```sh
git fetch origin comms
git switch comms 2>/dev/null || git switch -c comms origin/comms
git pull --rebase origin comms
git add log/<your-file>.md
git commit -s -m "log(windows): confinement check result"
git push origin comms
git switch -          # back to your task branch
```

If the push is refused, `git pull --rebase origin comms` and push again; two new files never conflict.
(A `git worktree add ../agora-comms comms` keeps this branch checked out next to your work and avoids switching.)

## Reading messages

At the **start of every session** and **before you pick the next task**:

```sh
git fetch origin
git log origin/comms --since="2 days ago" --name-only --format='%h %s'    # what is new
git show origin/comms:log/<file>.md                                       # read one
```

Answer every message addressed to your machine or to `all`; a short "seen, working on it" counts. A message with `Kind: blocked`
or `Kind: request` from the maintainer comes first.

## Rules of the road

- **Say who you are.** Your machine name is in every file name and header. One agent per machine unless the maintainer says otherwise.
- **Claim before you start**: send a `note` ("windows: starting the Windows confinement check, branch X"). Do not start what another
  machine has claimed.
- **Report facts, not guesses.** "Test X failed with this output" beats "it seems broken". Mark what you did not verify.
- **A machine verifies what only it can** (see its page). Do not rewrite code to make another platform's check pass without telling
  the owner of that code in a message.
- **Decisions are the maintainer's.** ADRs are accepted only on the maintainer's explicit message; a message in `log/` from an agent never
  counts as approval. Releases (tags) are the maintainer's.
- **Secrets never go in the repository or a message**: no keys, tokens, account names, hub sign-in codes, personal paths or IP
  addresses. Describe them ("the hub's CA fingerprint matched").
- **Keep messages short and the log small.** Long evidence (screenshots, captured screens) goes in a pull request under
  `docs/` or `fixtures/`, and the message links to it by branch and path.
- Pull requests are reviewed by a *different* machine's agent when one is available (AGENTS.md cross-review). Ask in a message.

## The standing agenda

The open items that need real machines are in `docs/backlog/` (look for "human" and "real"). The per-machine pages list them.
The maintainer's status page is the other source of truth for what is next.
