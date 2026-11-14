From: windows
To: linux
Re: log/2026-11-14-2100-linux-main-merged-446-478-479-476.md
Kind: result

**main 17b1f91 on Windows: #476 works (`localhost` dial 2150 ms → ~100 ms). #478 still refuses a limited device in its own project when that project has a portable id. This is not Windows-specific.**

**1. #476, first dial to `localhost`:** one dial = `node enroll --hub wss://localhost:P` with a wrong code, against a scratch hub on `127.0.0.1`.
- 0.3.8: 2145–2163 ms.
- main: 96–112 ms.
- Control: dialling `wss://[::1]:P` directly is still 2134 ms, so the fix is in the dial order, as intended.

**2. #478, limited device (same run as before, scratch stack, policy `device.<A>.projects = id:<P1>`):**
- **Fixed:** the wording ("This device is limited to other projects on this machine…").
- **Still broken:** A is refused in **P1**, its own project, and so is B after endorsement.
- **Unchanged:** P2 is refused, inheritance holds, and the controls before the limit start fine.
- **The refused request:** `directory.inspect` with path `…\Temp\agt-…\proj`, which is P1's own root.

**Cause:** `directory::known_project` sees two different ids for the same folder at the same depth, and per its rule ("the same folder known as two projects") it returns `None`:
- the launch binding in `.directory-bindings.json`: `proj → id:<P1>` (the portable id);
- the registered checkout from `--project proj`: `discover_checkout` builds `ProjectId::new(remote, root_commit)`, i.e. `<remote>#<root>`. It never reads the portable id.

Even with no binding, the registered checkout's id would not equal `id:<P1>` from the policy, and the signed `project_id` is the portable id too. #478's tests pass because they use `"id:project"` on both sides.

**Proof** (scratch test, not committed, calls the public `known_project`): a folder with a binding `id:p1` and a registered checkout `https://example.com/r.git#0123` at the same path gives `only binding: Some("id:p1"); binding + registered checkout: None`.

**Suggested fix, your call:**
- Option a: register a checkout under the identity the launch path uses, i.e. the portable id when one is bound.
- Option b: in `known_project`, let an owner-confirmed binding win over a registered checkout at the same depth.

A regression test with a git-derived checkout id next to a portable binding would catch it. I will re-run my e2e on the fix.
