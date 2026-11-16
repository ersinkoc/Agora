# PR #616 terminal-holder ADR review

Reviewed head: `8b015c3add56cb6da36350262ba9e9e181b884e9`.
Verdict: **APPROVE the accepted design document**. This is not approval of holder code: none is included in this PR. The maintainer's acceptance, including the review additions and deferral of periodic snapshots/cgroups, is already recorded explicitly. No repeated human acceptance request is needed.

The ownership split is consistent with autonomous nodes and separate domain/runtime state. The holder owns process/terminal lifetime, detached output and exit status; policy, approvals, detection, VT and mesh remain with the node. Passing terminal handles avoids adding an output/input relay in the attached hot path. A version mismatch keeps live agents untouched. The node-holder endpoint requires owner-only access plus the role-lock process identity, rather than trusting every same-user peer. Existing node confinement is carried into each launch. Clean-stop snapshot ordering, detached output, retained exit status and the stated crash limits are explicit.

The document already requires the important acceptance controls. Preserve these as implementation gates:

- Cancel and complete the holder's outstanding reads before transferring the buffer and terminal descriptors, so a byte is consumed by exactly one reader. A clean-stop VT snapshot must correspond to the node's final consumed byte, then detached bytes replay after it. Windows cancellation completion and partial reads need native tests.
- Test a descendant that outlives its leader, alongside stale/reused process identities. Decision 8's leader-identity validation must distinguish a mismatching replacement from an exited leader whose surviving members are still attributable. Do not substitute the current leader-only `Ledger::still_running()` for the required group-member behavior. If the implementation cannot safely establish that attribution, report the limit explicitly and request a narrow ADR clarification rather than signal an unverified group.
- On Linux/macOS, the already accepted no-node/holder-death exception remains an exception: ignoring SIGHUP or leaving the foreground group can preserve a process. Do not claim Windows kill-on-close semantics on Unix. The prescribed `still_running`/stop control must cover it without creating a duplicate resumed agent.
- Prove the owner-only endpoint and role-lock peer identity on each OS; reject concurrent/wrong node processes before transferring descriptors or accepting launch instructions. Avoid weakening those checks just because a peer has the same uid.
- Measure attached-path overhead and reattach of sixteen terminals as required by ADR-0022; keep a bounded per-terminal detached buffer and the deferred snapshot/cgroup work explicit.

Reviewed the complete ADR, the ADR-0075 interaction, the current ledger's leader-only process test and the architecture/performance requirements. `sh scripts/check-docs.sh` passes. The Windows experiments in the ADR are attributed to their author; no Windows/macOS holder execution or new holder implementation was tested by this reviewer.
