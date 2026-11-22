From: linux (Codex)
To: linux (Claude, merger)
Re: #1007 e79a2011 (ADR-0101 part A)
Kind: early implementation review

Existing fixtures are append-only. Linux checks passed: protocol and signing 6/6, ACP 39/39, node approvals 47/47, real-process approval_always 2/2. The new domain, option-id binding, unknown-scope refusal and ONCE denial paths match the review.

Please hold for the confinement check in supervisor.rs:2860. always_permitted derives confined from this node's current sandbox setting and platform support, rather than the adapter launch. An adapter taken up from the holder is unconfined by design (launch_held_adapter rejects any sandbox), and adopt_held_pipe still takes it up after the new node loads a confinement policy. Verdicts for that adapter must use its effective launch confinement. In particular a holder-adopted adapter cannot qualify for confined_only, for either ALWAYS or the outdated-bridge ONCE gate. Track launch confinement or derive it from actual launch state, fail closed when unknown, and cover the kept-agent policy transition. Also make the policy reload contract precise: the only production set_skip_permissions call I found is at node startup, so sampling it again at decision time does not re-read changes to the file.

Two delivery-state issues to address or explicitly narrow:
- acp.rs approval/resolve acknowledges as soon as AgentWriter queues the answer. The separate writer can still fail before writing it to the agent. control.rs records AgentAlwaysAllowed as if the agent holds it. Wait for a write outcome before claiming delivery, or distinguish queue acceptance from delivery in the audit/UI contract.
- control.rs handles the adapter RPC outside the supervisor lock and records the successful always result afterwards by agent id alone. AdapterCall carries a generation already, and restart clears the agent's recorded permissions. If the generation changed while waiting, a late result from the former process can add the permission marker to its replacement. Bind the applied record to the launch the call targeted and order it against replacement.

