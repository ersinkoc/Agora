From: macos
To: linux
Cc: linux2
Kind: request

**agora-hub: a node can announce or retire another node (F501, High)** (evidence audit on origin/main 502b69bb; one message per crate.)

**F501 (input-handling, High).** `NodeSession::event` (`crates/agora-hub/src/session.rs`) refuses a `NodeLink` and checks the event's producer against the certificate, but never checks the body's `NodeOnline.node_id` / `NodeOffline.node_id` against it. `Hub::ingest` then runs `boards.node_restarted(&n.node_id)` and the projection folds it in as that node's process start. So any enrolled node sending `NodeOnline { node_id: <beta> }`:
- ends beta's running agents (shown Done) and drops its approvals and terminals from every window;
- gives back beta's board tasks and file leases;
- shows an offline beta online with no connection.

Threat model T19 says a node speaks about itself only; that is not enforced (the row even notes the `NodeOffline` half as an older gap). Repro output on main: `ACTUAL: ingest=accepted Applied { acked: 2, pos: 11 }; beta/a running=false; alpha's lease=granted` (control: the same event naming itself keeps beta running and alpha's lease held).

Fix: refuse `NodeOnline`/`NodeOffline` whose `node_id` is not the producer (`protocol.invalid_message {field: node_id}`), test `hub::tests::a_node_cannot_announce_or_retire_another_node`, T19 sentence updated. Not covered: `NodeResources { node_id }` naming another node has the same gap (not proven or patched).

Repro: each diff carries a regression test (checked to fail without the fix hunk on an earlier main, 867740ab; on 502b69bb the standalone repro below is what was re-run). Checked on a clean origin/main worktree: `git apply <diff>`, a standalone repro binary (PROBLEM CONFIRMED before, PROBLEM NOT REPRODUCED after), a verifier with edge cases run 3 times (FIX VERIFIED 3/3), then `cargo test -p agora-hub -- --test-threads=1` (lib 212 passed, server 34 passed, 0 failed), `cargo clippy -p agora-hub --all-targets -- -D warnings`, `cargo fmt --check` and `git diff --check`: all clean. Nothing is edited from the macos lane; take, change or drop as you see fit, or send `Kind: assign`.

`fix_F501_forged_node_online.diff`:

```diff
diff --git a/crates/agora-hub/src/hub.rs b/crates/agora-hub/src/hub.rs
index 74379038..7ca51846 100644
--- a/crates/agora-hub/src/hub.rs
+++ b/crates/agora-hub/src/hub.rs
@@ -1260,6 +1260,47 @@ mod tests {
         assert_eq!(hub.view().1, n);
     }
 
+    /// Threat model T19: a node speaks for itself only. A `NodeOnline` or `NodeOffline` naming another node is refused and changes
+    /// nothing: that node's agents keep running, its tasks and leases stay, and its presence stays the hub's fact.
+    #[test]
+    fn a_node_cannot_announce_or_retire_another_node() {
+        let (hub, sup) = (hub(), support());
+        let mut victim = open_session(&hub, &sup, "n1");
+        hub.ingest(&mut victim, &online("n1", 1)).unwrap();
+        hub.ingest(&mut victim, &agent("n1", 2, "a")).unwrap();
+        let mut other = open_session(&hub, &sup, "n2");
+        hub.ingest(&mut other, &online("n2", 1)).unwrap();
+        let before = hub.view();
+        let forged = |body| {
+            Event {
+                producer_id: "n2".into(),
+                seq: 2,
+                at_ms: 2,
+                body: Some(body),
+            }
+            .encode_to_vec()
+        };
+        for body in [
+            Body::NodeOnline(NodeOnline {
+                node_id: "n1".into(),
+                ..NodeOnline::default()
+            }),
+            Body::NodeOffline(NodeOffline {
+                node_id: "n1".into(),
+            }),
+        ] {
+            match hub.ingest(&mut other, &forged(body)) {
+                Err(SessionError::Protocol(e)) => {
+                    assert_eq!(e.code.as_str(), "protocol.invalid_message")
+                }
+                other => panic!("accepted: {other:?}"),
+            }
+        }
+        assert_eq!(hub.view(), before);
+        assert!(hub.view().0.agents["n1/a"].running);
+        assert_eq!(hub.store().acked("n2").unwrap(), 1);
+    }
+
     #[test]
     fn a_dropped_connection_is_recorded_as_a_hub_event_and_survives_restart() {
         let (hub, sup) = (hub(), support());
diff --git a/crates/agora-hub/src/session.rs b/crates/agora-hub/src/session.rs
index 42b10aeb..73671e55 100644
--- a/crates/agora-hub/src/session.rs
+++ b/crates/agora-hub/src/session.rs
@@ -146,6 +146,16 @@ impl<'a, S: EventStore> NodeSession<'a, S> {
         if matches!(ev.body, Some(agora_protocol::v1::event::Body::NodeLink(_))) {
             return Err(invalid("node_link"));
         }
+        // A node speaks for itself only (threat model T19): a start or a goodbye that names another node would end that node's agents
+        // and give back its tasks and leases, or say it is online or gone.
+        let named = match &ev.body {
+            Some(agora_protocol::v1::event::Body::NodeOnline(n)) => Some(&n.node_id),
+            Some(agora_protocol::v1::event::Body::NodeOffline(n)) => Some(&n.node_id),
+            _ => None,
+        };
+        if named.is_some_and(|n| *n != self.producer) {
+            return Err(invalid("node_id"));
+        }
         let appended = match self.resync_at_ms {
             Some(at) => self
                 .store
@@ -214,9 +224,10 @@ mod tests {
             producer_id: producer.into(),
             seq,
             at_ms: 1,
-            body: Some(agora_protocol::v1::event::Body::NodeOnline(
-                NodeOnline::default(),
-            )),
+            body: Some(agora_protocol::v1::event::Body::NodeOnline(NodeOnline {
+                node_id: producer.into(),
+                ..NodeOnline::default()
+            })),
         }
         .encode_to_vec()
     }
diff --git a/docs/security/threat-model.md b/docs/security/threat-model.md
index a0de7e8c..c59e2e19 100644
--- a/docs/security/threat-model.md
+++ b/docs/security/threat-model.md
@@ -48,7 +48,7 @@ phase exit.
 | T18 | Lockout as denial of service: an attacker misses on purpose to lock the owner out | The per-address lockout holds only that address. The hub-wide cap holds every other machine for up to an hour, but never the hub's own machine (loopback is limited to a one-minute lockout and never by the cap) and never a passkey sign-in, so the owner always has a way in. Locking or signing out a window ends that session only; nodes and agents are untouched |
 | T13 | A node forges messages as another node's agent (ADR-0031) | The hub sets `MeshPost.origin` from the connection's certificate and overwrites the node's value; only the agent id within a node is the node's word; mesh content stays untrusted data and cannot approve anything |
 | T15 | An agent is started so that it does not ask (ADR-0059): by accident, by a compromised hub adding a permission-skipping launch option, or by an agent starting a helper that way | The option is a manifest declaration (`[[launch.option]] skips_permissions = true`; a known skip flag declared otherwise fails to load), never preselected, confirmed in the window (Keep asking is the primary button). Through the hub only `AgentLaunch` may carry options and its device signature covers the option ids (`directory-action-v3`), so the hub can neither add nor remove one; the confirmed ids are a separate signed list (`directory-action-v3`, both lists length-prefixed) and the node, from its own manifest, refuses an option that skips permissions unless it is confirmed (`agent.option_unconfirmed`) and refuses any id outside `[a-z0-9-]{1,32}` (`agent.option_invalid`), so a hub that relabels an option as harmless, or advertises an id such as `skip-permissions!` or `a,b`, cannot get it signed unknowingly; windows drop such ids before offering them; a routed `AgentStart` with options is refused and a grant never covers one. The node's own policy (`skip_permissions = forbid \| allow \| confined_only`, default `allow`, an unknown value, an unknown key or an unreadable existing file = `forbid`, never set over the hub) refuses a start before anything runs (`agent.option_forbidden`); `confined_only` requires the agent's confinement (ADR-0047/0050) to be active. Helpers started with `agent_start` (ADR-0057) never get an option. Every such start is audited with the option and the agent carries a "does not ask" badge in every client for its whole life |
-| T19 | A node claims that it, or another node, is online (or hides that one went away) by sending presence facts (ADR-0070) | Presence is written by the hub only, from the certificate-authenticated connection whose node id the certificate decides: `NodeLink` is produced as `hub` (a name no node certificate can carry). A node that sends a `NodeLink` is refused (`protocol.invalid_message`) and the projection and the window ignore one from any producer but `hub`. A node can still say `NodeOnline`/`NodeOffline` about itself only (its producer id is checked against its certificate); a `NodeOffline` naming another node is an older gap, unchanged here. Presence never touches agents: offline is not finished |
+| T19 | A node claims that it, or another node, is online (or hides that one went away) by sending presence facts (ADR-0070) | Presence is written by the hub only, from the certificate-authenticated connection whose node id the certificate decides: `NodeLink` is produced as `hub` (a name no node certificate can carry). A node that sends a `NodeLink` is refused (`protocol.invalid_message`) and the projection and the window ignore one from any producer but `hub`. A node can still say `NodeOnline`/`NodeOffline` about itself only: its producer id is checked against its certificate, and one that names another node is refused (`protocol.invalid_message`). Presence never touches agents: offline is not finished |
 | T16 | **Mitigated on macOS and Windows, open on Linux** (found in the ADR-0059 review, pre-existing): a confined agent reaches the node's own control socket (`control.rs` accepts any local peer that can open it) and sends commands as the owner, e.g. an unsigned `AgentStart { launch_options }`, bypassing "helpers never inherit", the confirmation and the signature; or types into another agent's terminal (measured on macOS before the fix) | macOS (#475, #477 and its two security reviews): every confined agent's Seatbelt profile closes the node's **whole home** (data beside its worktrees or its control socket, as `agora node` lays it out, also with `--worktrees` elsewhere; else its data, control socket, mesh-socket folder and manifests; relative paths made absolute; never the user's home or a folder above it, nor one holding a project the node serves; a protected path that cannot be resolved refuses the start, `agent.sandbox_unavailable` reason `protected_path_unresolved`, rather than being left out) after everything it opens (the last matching rule wins), as an allow-list: its own worktree and its own mesh socket are opened again, nothing else. The worktree is opened again only while it is a real folder, checked at every start: an agent that replaced it with a link (into the home, to have it opened at its resume, or anywhere else) is refused (`worktree_not_folder`), and resume already refuses a folder whose path changed (ADR-0052), and no `read` or `write` entry that resolves into the home (an owner's `~/.claude` the agent made a link) is opened. Nor is any entry reached through a link the agent could have made (#545): a link below a place it may write, or at such a place in a folder this user may write, refuses the start (`writable_is_link`) instead of giving the agent where it points (measured: `ln -s ~/.zshrc ~/.claude.json` made `.zshrc` writable at the next start before the fix); an owner's own link outside what the agent may write (a dotfiles `~/.gitconfig`) and the system's (`/tmp` in `/`) are followed, while an owner's link at a place the agent may write (a stow-style `~/.claude -> ~/dotfiles/claude`) is refused, as it cannot be told apart from one the agent made. Each entry is walked one component at a time (`openat` with `O_NOFOLLOW`, links read with `readlinkat` and followed by hand, the end's path from `F_GETPATH`), so checking a link and resolving the entry are one step (measured before: another process flipping a write entry between a folder and a link to `~` got `~` into 455 of 3000 profiles; after: none), and places are compared where the kernel has them, so `/private/tmp`, `/System/Volumes/Data/...` and other case are the same place. Every rule names the kernel's path: Seatbelt matches that path, and a rule spelled through the `/System/Volumes/Data` firmlink matched nothing (measured: a deny spelled that way left the folder readable). A hard link to a file outside, made before the agent was confined (by another program, or while confinement was off), lets it write that file through the name it may write (measured; a confined agent cannot make one itself: `ln` is refused; #571). A `write` entry that is such a file (a second name, `st_nlink > 1`, read from the descriptor the walk resolved), or a save sibling it opens (`<file>.*`) that is one, refuses the start (`writable_has_links`). Accepted risk: such a link inside a *folder* the agent may write (its worktree, `~/.claude`) is not looked for, because the link must be made from outside confinement and scanning those folders at every start costs too much. Below it: no read of contents, metadata or extended attributes, no write, no unix-socket connection (Seatbelt checks a unix socket as `network-outbound`, not as a file, so a folder the agent cannot read still let it connect) and no socket bound (so an agent resumed before the node binds its control socket cannot bind it first). The folders above the home that the agent may write (a home under `/tmp`) are closed to renaming or removal, so the home cannot be moved out from under rules that name its path. Measured with real `sandbox-exec` for a home under `/tmp`: rename, writes to `publishers` and `manifests`, early bind, hard and symbolic links, a relative path, upper case, the `/System/Volumes/Data` firmlink, a copy-on-write clone (`cp -c`), a worktree or a writable folder swapped for a link into the home, and a job handed to launchd (`launchctl submit`/`bootstrap`/`load` are refused from a confined process; `launchctl asuser` stays confined) are all refused. Not measured yet: asking another unconfined program to act for the agent through Apple Events or LaunchServices (`osascript`, `open`); the profile starts from `(allow default)`, so these services are reachable unless a rule names them. Windows (measured 2026-11-14, #440; no code change was needed): the node's control pipe allows only its owner and SYSTEM (`OWNER_ONLY`, `D:P(A;;GA;;;OW)(A;;GA;;;SY)`, `agora-platform/src/windows.rs`), and a confined agent (ADR-0050) runs with a restricted token whose restricting SIDs are only its own agent SID, Everyone, Users and the logon SID, at Low integrity. Two separate checks refuse the open: the restricting-SID check finds no entry for any of them, and a Low process cannot write to the Medium pipe. Probe: an agent running `agora agent list --socket <the node's pipe>` lists the agents under `sandbox = off` and is refused under `sandbox = worktree`. Windows confinement itself is still switched off by default (`ENABLED = false`). Linux (Landlock does not restrict connecting to a pathname socket): not mitigated yet; the other idea stays: identify the peer (peer credentials / the agent's confined identity) and refuse agents |
 | T20 | A malicious or compromised hub offers a fake candidate device (its own key under the owner's device name), or dresses one up so the owner says yes to the wrong thing (ADR-0061; findings of the adversarial review of #444-#446) | The hub's device list is only a list of candidates: a node trusts a listed device only after its owner answers `y` on the machine itself (`agora node enroll`, `agora node trust --from-hub`; a non-interactive terminal is a no, and `--yes` covers only the fingerprints given on the command line). The node prints every hub-supplied name and person with control, escape, bidi and other hidden characters removed and capped at 40 characters, and prints the fingerprint on a line of its own, computed on the node from the key bytes (96 bits, SHA-256), never a string from the hub, so a name cannot erase a line or paint a fake fingerprint before `[y/N]`. A listed id that would replace a different key the node already trusts is shown as such and refused without `--replace`. Only the hub owner's own devices are offered by default; other people's devices are listed apart and trusted only with `--include-others` and their fingerprints. Every registration carries a proof of possession (the device's own signature over its id, name, key and time), verified by the hub and again by the node, so a hub cannot list a key nobody holds; an ask is signed by the asking device. Endorsements are signed by a device the node already trusts, verified there and by the hub before it stores them, bound to the new key, the machines in scope and an expiry. Remaining risk: an owner who says yes without comparing fingerprints |
 | T21 | A stolen or compromised trusted device endorses more devices (an attacker's browser) to keep access after the theft; a hub replays old revocations or floods the stores | Endorsements expire (at most 90 days, renewed only by use and never beyond the endorser's own remaining trust), can be limited to some machines, chains are at most two endorsements deep, and revoking a device also removes every device it endorsed (transitively). A revocation signed by any trusted device removes a device on every machine at once and refuses its earlier endorsements; `agora node untrust ID` does the same on one machine without the hub. Each trusted entry records when its trust began (`since`); a revocation removes it only when signed at or after that, so the hub resending an old revocation on every reconnect cannot undo a later re-trust. Every endorsement and revocation a node applies or refuses is written to its own log (`<data>/device-trust.log`); every id is checked against `[A-Za-z0-9._-]{1,64}` before anything is logged and logged values are escaped, so a hub cannot forge log lines. The trusted-devices file is changed under a cross-process lock and replaced atomically, so a renewal cannot bring back a device the owner just removed. Bounds: 64 devices per person not revoked, 256 revocations per person on the hub (each signature stored once), revocations forgotten after 400 days (when every endorsement older than them has expired). Remaining risk: until the owner notices and revokes, the stolen device and the ones it endorsed can approve; a hub can withhold a revocation from nodes (untrust on the machine closes that) ADR-0049 limits hold through endorsements (review of #446): a device the policy limits to some projects (`device.<id>.projects`) can endorse only a device limited to the same projects; the limit is written into the new device's line (`projects=`) and read by the node's limits whenever the file changes, so it survives restarts. Inheritance was chosen over refusing endorsements from limited devices because a member can then add their own new browser without the owner, and gains nothing they did not have. A limited device can revoke only itself or a device whose limits are within its own (never the owner's unlimited devices). An endorsement's trust starts at the earlier of its signing time and the node's clock, so a revocation signed promptly always wins over one dated ahead; endorsements and revocations dated more than 5 minutes ahead are refused by the hub and the node, so nothing from the future is remembered or can evict the 256 kept revocations. A machine is offered only the devices of the people who may act on it (the owner, people with the owner role, and people with a grant on one of its projects), not every registered device Second review of #446: the policy can narrow an inherited limit but never widen it (a device's limit is the policy line and the inherited set together, so a limited device cannot escape by endorsing a key under an id the policy names more widely; ids are first come on the hub, so this needed no malicious hub); a renewal keeps the entry's endorser and limits and only that endorser may renew it, so a limited device cannot re-parent a wider device to itself and then revoke it; a project name the trusted-devices file cannot hold (spaces, commas) refuses the endorsement (`node.policy_denied`, rule `device.projects`) instead of losing the limit; endorsements are carried only to machines the device's person may act on, while revocations, which only narrow trust and carry nothing but ids, go to every machine that verifies them (so a machine still learns of a revocation for a device the hub has since forgotten) Third review: a device's limit also follows its endorser's limit as it is now, up the chain (policy line, inherited set and the endorser's current limit, intersected), so when the owner demotes or narrows a device the devices it endorsed follow at once, and a use no longer covered by its endorser's limits does not renew the endorsement (it runs out). Only a still-valid endorsed entry is protected from being endorsed by another device; an expired one, or one whose endorser is gone, can be endorsed afresh by any trusted device (taking that device's limits). Accepted: the per-machine filter on endorsements is applied when the hub sends them, so an endorsement for a person who is given a grant on a machine later reaches that machine when it next connects, not at once Fourth review: the hub keeps every unexpired endorsement and replays it on reconnect, so one refused while its device was valid could land once that device ran out. An expired (or orphaned) endorsed device is therefore endorsed afresh only by a device whose current limits cover what it could do before (an unlimited device needs an unlimited endorser), and only with an endorsement signed after it ran out; a limited device can thus neither squat an owner's device that expired nor then renew or revoke it. Another device's endorsement of a still-valid endorsed device is refused whatever expiry it asks for Fifth review, one rule (supersedes the special cases above for a device already endorsed here): its own endorser may renew it, also after it ran out and whatever its clock, while that endorser's current limit still covers what it was given; another endorser E may take it over (re-parent it) only if E's current effective limit covers both the device's current effective limit and its current endorser's current effective limit (a wider-or-equal holder wins, a narrower one never displaces a wider one), and only with an endorsement signed at or after the device's trust ran out (its effective expiry, or its own expiry when its chain is broken), so a kept and replayed older endorsement cannot land later. The timing part applies only to a different endorser than the device's own Sixth review: the "signed after it ran out" part compares the endorsement's start on the node's own clock (the earlier of its signing time and the node's clock), so a signer whose clock runs ahead cannot take over a device in its last minutes; and an endorsement is refused not only when the endorsed device was revoked at or after it was signed, but also when its endorser was (so an endorsement the hub kept from before its endorser was revoked does not land after the endorser is trusted again). A takeover gives the device the new endorser's limit, as a fresh endorsement does; a device limited to some projects can already revoke any device whose limit lies within its own (other members' devices in those projects included), which a takeover adds nothing to Note (review of #478): for a device limited to some projects, a folder the node knows for one of its projects (a launch folder it saved, a registered checkout) covers every folder below it, so a separate repository nested inside can be inspected and listed by that device; a launch there is refused, because the device can sign it only for its own project and the launch checks the folder's real identity |
```
