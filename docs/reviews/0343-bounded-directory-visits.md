# Bounded directory visits (#343)

The first #333 implementation materializes a whole directory before its watcher checks the 50,000-entry limit.
An 80,000-entry directory therefore costs 80,000 metadata reads per scan despite retaining only 50,000 records.
The pre-#333 streaming watcher stops at 50,000 metadata reads. The handle check that closes #330 must remain.

`agora_platform::visit_dir` visits one entry at a time through the open, checked directory handle. The callback's
`ControlFlow::Break` stops before the next entry is described. `list_dir` remains a collection wrapper for callers
that need a complete directory. The watcher uses the visitor, stops at its existing record limit, and does not
open another queued directory once that limit is reached. Root verification, no-follow entry metadata, `.git`
exclusion, sorting of directory responses and watcher batches remain the same.

The limit counts recorded non-directory entries, as before; it does not bound the number of traversed directories.
This change adds no schema, dependency, policy, permission or authentication behavior.

## Regression verification

The new platform test changes every later file's size from the first callback. Later callbacks must observe the
new sizes regardless of directory order. A variant that first collects metadata into a vector fails this test;
the streaming implementation passes. The test also stops after one callback and then lists through the caller's
original handle again, checking both early termination and descriptor ownership. Its fixture is exclusively
created before a cleanup owner is registered, so pre-existing paths are never adopted or removed.

Existing `files` folder-swap and watcher-swap tests check that traversal still refuses locations outside the root.
Full workspace tests (including doc tests), workspace Clippy with warnings denied, formatting, locale lint and
repository docs checks pass on Ubuntu with the streaming source restored after the negative control.

```sh
cargo test -p agora-platform list_dir_tests -- --nocapture
cargo test -p agora-node files:: -- --nocapture
```

## Linux filesystem measurement

Ubuntu, kernel 7.0.0-42-generic, debug build, one private directory with 80,000 empty file names (two seed inodes,
40,000 hard links each). Only disposable test data was used. Eager and streaming binaries were built from the
same #333 base and toolchain; only this follow-up's source differs. No Cargo builds ran during measurement.

For the syscall and RSS probes, the entire measured workload is:

```rust
let files = agora_node::files::Files::open(root, 1024)?;
let mut watcher = files.watch();
assert!(watcher.poll().is_empty());
```

`strace -f -c -e trace=newfstatat,statx PROBE DIRECTORY` counts metadata syscalls. A separate
`/usr/bin/time -f 'max_rss_kb=%M elapsed_s=%e' PROBE DIRECTORY` invocation measures process peak RSS.
The fixture is prepared beforehand, and the probe neither creates nor removes files. A third invocation warms
the watcher with three polls, then sorts ten timed polls and reports the nearest-rank p95 (the tenth sample).

| Measurement | Eager #333 | Streaming #343 |
| --- | ---: | ---: |
| Metadata syscalls, initial scan + one poll | 160,000 | 100,000 |
| Process peak RSS, separate run | 17,908 KiB | 12,920 KiB |
| Warm poll p95, ten samples | 283.658 ms | 265.642 ms |

The syscall result proves the watcher once again stops describing entries at its budget. RSS and latency are
single-machine fixture measurements, not general capacity or interaction-budget claims. A watcher still records
50,000 paths; the removed allocation is the directory-wide intermediate vector. The private fixture was removed
after collecting evidence, without touching installed services, credentials or real agents.

## Windows integration

The initial follow-up inherited #333's Windows `Unsupported` stubs. The integrated implementation opens a directory
with `FILE_FLAG_BACKUP_SEMANTICS`, then verifies its handle using the existing root check. Enumeration uses
[`NtOpenFile`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntopenfile), with an empty name
relative to the held directory, to obtain an independent synchronous handle to the same filesystem object. Explicit
`FILE_DIRECTORY_FILE` and `FILE_SYNCHRONOUS_IO_NONALERT` options select directory access and completed queries; the path
is never looked up again. `FILE_OPEN_REPARSE_POINT` prevents that open from following a newly planted reparse point.

[`NtQueryDirectoryFile`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntquerydirectoryfile)
requests one `FILE_DIRECTORY_INFORMATION` at a time. A fixed, aligned buffer holds the record; its completed byte
count and UTF-16 name length are checked before reading the name. A callback that stops causes no further query.
Reparse entries are never traversed. The additional `windows-sys` WDK feature exposes the existing dependency's
documented single-entry API; no new dependency or policy is added.

A deterministic regression opens a folder, renames it, and replaces the old path with a link to an outside folder.
The held handle still lists the original contents while a new open lists the replacement. On Windows it uses a
junction, which does not require symbolic-link privileges, so that test cannot silently skip on an ordinary CI account.
The lazy-metadata, early-stop and repeat-enumeration tests also run on Windows.

This follow-up remains stacked on #333. Cross-compilation checks supplement native CI; they do not substitute for
running these tests on Windows. Codex coordinated the implementation in #333 and claimed the follow-up in #343.
