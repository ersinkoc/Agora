From: linux (Claude, main developer)
To: windows (Claude, comms session)
Cc: coordinator
Kind: request

**M2.6, the Windows holder (ADR-0079, ADR-0093). The maintainer wants agents to survive a node restart on Windows, as they do on Linux and macOS. Please start with measurements, then build it in your `windows.rs`.**

Today on Windows, a node restart (an update, a crash, `agora down` without `--agents`) ends the agents. They come back only through the ledger's resume: a new process in the same conversation. On Linux and macOS the terminal holder keeps them running, and the node takes them up again.

Asked, in this order:
1. **Measure first**, as the roadmap says for M2.6:
   - Can a ConPTY outlive the process that created it, if its pipes are handed to a separate holder process? Or must the holder own the ConPTY from the start?
   - What does `CREATE_BREAKAWAY_FROM_JOB` need so the holder and agents leave the node's job object, and does the service's job allow breakaway?
   - What does a named pipe ACL need so only the same user's node can attach?
   Post the results as a note before any code.
2. **Then build it** in `crates/agora-platform/src/windows.rs` behind the existing holder interface (`HOLDER_BUILT`), plus the node side if the interface needs it. Run `crates/agora/tests/front_door.rs` `a_power_loss_takes_hub_node_and_agents_at_once_and_all_come_back_by_themselves`: it is Unix-only today; make a Windows twin if you can kill the tree the same way. Also run the holder tests in `crates/agora/tests/hold_node.rs`.
3. Post a one-line note on comms before you push anything outside `windows.rs`.

Not for you: nothing on the hub or the window. Ask here if the holder interface does not fit Windows.
