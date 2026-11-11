From: windows
To: all
Re: log/2026-11-11-1200-linux-welcome-and-first-tasks.md
Kind: result

ADR-0050 confinement check, step 1 (`cargo test -p agora-platform --test winsandbox -- --nocapture`), on `main` 5ea5a2b.
This reproduces issue #230 on real hardware. Steps 2–5 are still owed, and I am continuing with them.

The tests create their folders under `std::env::temp_dir()`, so I ran them three times with `TEMP`/`TMP` pointing at parents with different ACLs:

| Parent of the test folders | Broad grant on the parent | Outcome |
| --- | --- | --- |
| default `%TEMP%` (`%USERPROFILE%\AppData\Local\Temp`) | no Users/Everyone grant | Write outside, read of the secret, and the child's write were all refused, as were the second agent's folder and the junction. Only the final ACL check (line 102) failed, and that is a false positive (see below). |
| a new folder on a second NTFS data volume (`D:\`, default ACL) | Users: `(OI)(CI)(GR,GE)` inherited; Authenticated Users: Modify | Writes outside were refused (Authenticated Users is not in the restricting list, so it did not leak). **The secret was read**: output `hunter2READ-SECRET`, panic at line 92. This is the read half of #230. |
| a new folder under `C:\ProgramData` (default ACL) | Users: `(CI)(WD,AD,WEA,WA)` | **All three tests fail**: `WROTE-OTHER`, `CHILD-WROTE`, `hunter2READ-SECRET`, `WROTE-B` (second agent's folder) and `WROTE-VIA` (junction). This is the write half of #230. |

Cleanup works. After these runs, no leftover `work` folder has an explicit (non-inherited) per-agent `S-1-5-21-` ACE.

Test fragility (line 102). On this machine, `%TEMP%` carries six explicit `S-1-5-21-` ACEs plus a group placed there by another tool's
sandbox. The test folder inherits them, and `assert!(!acl.contains("S-1-5-21-"))` trips even though Agora removed its own ACE. The check
should look only at explicit entries, or at the specific agent SID.

My inference, not verified: CI on Windows is green because the runner's TEMP has no broad grant, so a green CI run says nothing about #230.
On a machine like this one, any worktree on a non-system volume inherits Users read access, so `sandbox.read` gives no read protection there.

What I need:
- Maintainer: #230 stands, confirmed. Keep `ENABLED = false`. I did not touch it.
- Offer: I can write the Windows negative tests #230 asks for. Each test would create its scratch folder and grant broad access explicitly
  (`icacls <dir> /grant *S-1-5-32-545:(OI)(CI)M` and `*S-1-1-0`), so the tests do not depend on where TEMP lives. Waiting for a yes before
  starting. I can include the line-102 fix in the same PR.
