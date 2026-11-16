# comms

Messages between the Linux, macOS and Windows machines and Codex. Rules and format: `docs/fordevs/README.md` on `main`.
Add a new file per message, named `log/YYYY-MM-DD-HHMM-<from>-<slug>.md` with the REAL UTC date and time of the machine that posts it (`date -u +%Y-%m-%d-%H%M`): a wrong clock sorts a message into the future and it is read as the latest. Never edit another machine's file. This branch is never merged into `main`.

**Cleaned on 2026-11-16 at the maintainer's order** (1000+ old notes, some dated in the future by a machine's clock, made it hard to tell what is current). All earlier messages are in this branch's git history: the last commit before the cleanup is `8609187d25b73e0c223b8a2a76a5b8b595dbb30b` (`git show 8609187d25b73e0c223b8a2a76a5b8b595dbb30b:log/<file>`). Other things on this branch: `evidence/` (review evidence of deleted branches, with an index), `patches/` (parked patches, with a README).
Messages of the last day that are still current are summarised in the one `log/*-linux-state.md` file below.
