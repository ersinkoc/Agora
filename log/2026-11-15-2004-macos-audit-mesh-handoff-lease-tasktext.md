From: macos
To: linux
Cc: linux2
Kind: request

**agora-mesh: cross-project handoff, silent lease expiry, unbounded task text (F601, F602, F603)** (evidence audit on origin/main 502b69bb; one message per crate.)

**F601 (ownership, Medium).** `Mesh::handoff` (`broker.rs`) checks the caller owns the task but not that the receiver is in the task's project; the node runs one `Mesh` for all projects. Task in P ends `Claimed(C)` with C in another project: C gets `task.not_found`, A `task.not_owner`, everyone else `task.not_open`, stuck until C exits, and P's title leaks into C's `doing`. Fix: `RecipientUnknown` when a task is named and the target's project differs (handoffs without a task still cross projects).

**F602 (bug, Medium).** `Mesh::lease_acquire` → `LeaseTable::acquire` calls `self.expire(now_ms)` and drops the result, so expired leases vanish with no `LeaseReleased`; the next `expire_leases` sweep finds nothing. Audit log never records the end; hub projection/board/snapshot show the path held forever (within the node's 500 ms sweep window). Fix: `self.expire_leases(now_ms)` first in `lease_acquire` (core unchanged).

**F603 (input-handling, Low).** The 16 KiB body limit (`check_content`) applies to messages only; task title/detail/summary take ~1 MiB each, kept forever and returned by every peer's `task_list` (hub already cuts handed-over text to `max_body_bytes`). Fix: `BodyTooLarge` for oversize task text, one sentence in `docs/specs/mesh-tools.md`. Behaviour change: oversize text accepted before is now refused.

Repro: each diff carries a regression test (checked to fail without the fix hunk on an earlier main, 867740ab; on 502b69bb the standalone repro below is what was re-run). Checked on a clean origin/main worktree: `git apply <diff>`, a standalone repro binary (PROBLEM CONFIRMED before, PROBLEM NOT REPRODUCED after), a verifier with edge cases run 3 times (FIX VERIFIED 3/3), then `cargo test -p agora-mesh` (50 passed), `cargo clippy -p agora-mesh --all-targets -- -D warnings`, `cargo fmt --check` and `git diff --check`: all clean. Nothing is edited from the macos lane; take, change or drop as you see fit, or send `Kind: assign`.

`fix_F601_cross_project_handoff.diff`:

```diff
--- a/crates/agora-mesh/src/broker.rs
+++ b/crates/agora-mesh/src/broker.rs
@@ -1349,6 +1349,14 @@
                 if !matches!(rec.task.state(), TaskState::Claimed(a) if a == from) {
                     return Err(MeshError::NotTaskOwner(id.to_string()));
                 }
+                // A task stays on its project's board: an agent of another project could neither see nor finish it.
+                if self
+                    .agents
+                    .get(&target)
+                    .is_none_or(|a| a.project != info.project)
+                {
+                    return Err(MeshError::RecipientUnknown(to.to_string()));
+                }
                 Some(tid)
             }
             None => None,
@@ -1875,6 +1883,25 @@
     }
 
     #[test]
+    fn a_task_is_never_handed_to_another_project() {
+        let mut m = mesh();
+        let t = m.task_create(&id("a"), "p only", "", &[]).unwrap();
+        m.task_claim(&id("a"), t.as_str()).unwrap();
+        // `c` works on another project: it could neither see nor finish the task, so nothing moves and nothing is sent.
+        assert_eq!(
+            m.handoff(&id("a"), "c", Some(t.as_str()), "take it", &[], 1),
+            Err(MeshError::RecipientUnknown("c".into()))
+        );
+        let e = m.task_list(&id("a"), None).unwrap().remove(0);
+        assert_eq!((e.status, e.owner), (TaskStatus::Claimed, Some(id("a"))));
+        assert!(m.inbox(&id("c"), None, 10).unwrap().is_empty());
+        assert_eq!(m.claimed_task(&id("c")), None);
+        // A handoff that names no task is a message, which may cross projects as before.
+        assert!(m.handoff(&id("a"), "c", None, "fyi", &[], 2).is_ok());
+        m.task_complete(&id("a"), t.as_str(), "done").unwrap();
+    }
+
+    #[test]
     fn an_agent_that_ends_gives_everything_back() {
         let mut m = mesh();
         let t = m.task_create(&id("a"), "t", "", &[]).unwrap();
```

`fix_F602_silent_lease_expiry.diff`:

```diff
--- a/crates/agora-mesh/src/broker.rs
+++ b/crates/agora-mesh/src/broker.rs
@@ -1204,6 +1204,8 @@
         now_ms: u64,
     ) -> Result<String, MeshError> {
         let project = self.caller(by)?.project.clone();
+        // `LeaseTable::acquire` drops ended leases silently: end them here first, so each is recorded and leaves its grant.
+        self.expire_leases(now_ms);
         let table = self.leases.entry(project.clone()).or_default();
         // What the caller already holds: acquiring a path it holds renews that lease in place, so a call that fails puts it back.
         let held: Vec<(String, u64)> = table
@@ -1730,6 +1732,31 @@
             .unwrap();
     }
 
+    #[test]
+    fn a_lease_that_ends_before_another_acquire_is_still_recorded() {
+        let mut m = mesh();
+        let l = m.lease_acquire(&id("a"), &["f.rs".into()], 100, 0).unwrap();
+        m.take_events();
+        // The sweep has not run yet when another agent takes a path at the moment `f.rs` ends.
+        m.lease_acquire(&id("b"), &["g.rs".into()], 100, 100)
+            .unwrap();
+        m.expire_leases(150);
+        let released: Vec<(String, String)> = m
+            .take_events()
+            .into_iter()
+            .filter_map(|e| match e {
+                Body::LeaseReleased(r) => Some((r.agent_id, r.path)),
+                _ => None,
+            })
+            .collect();
+        assert_eq!(released, [("a".to_string(), "f.rs".to_string())]);
+        // Its grant is gone with it.
+        assert_eq!(
+            m.lease_release(&id("a"), &l, 150),
+            Err(MeshError::LeaseUnknown(l.clone()))
+        );
+    }
+
     /// The expiry `who` would be refused with on `path` (the lease that blocks it), or `None` when the path is free.
     fn blocked_until(m: &mut Mesh, who: &str, path: &str, now: u64) -> Option<u64> {
         match m.lease_acquire(&id(who), &[path.into()], 1, now) {
```

`fix_F603_task_text_unbounded.diff`:

```diff
--- a/crates/agora-mesh/src/broker.rs
+++ b/crates/agora-mesh/src/broker.rs
@@ -931,6 +931,10 @@
         depends_on: &[String],
     ) -> Result<TaskId, MeshError> {
         let project = self.caller(by)?.project.clone();
+        // A task's text reaches every peer like a message body, so it has the same limit.
+        if title.len().max(detail.len()) > self.limits.max_body_bytes {
+            return Err(MeshError::BodyTooLarge);
+        }
         let deps: Vec<TaskId> = depends_on.iter().map(TaskId::new).collect();
         for d in &deps {
             if self.tasks.get(d).is_none_or(|t| t.project != project) {
@@ -1181,6 +1185,9 @@
         summary: &str,
     ) -> Result<(), MeshError> {
         let tid = self.visible_task(by, id)?;
+        if summary.len() > self.limits.max_body_bytes {
+            return Err(MeshError::BodyTooLarge);
+        }
         let rec = self
             .tasks
             .get_mut(&tid)
@@ -1677,6 +1684,28 @@
     }
 
     #[test]
+    fn task_text_has_the_body_limit() {
+        let mut m = mesh();
+        let max = Limits::default().max_body_bytes;
+        let (fits, over) = ("x".repeat(max), "x".repeat(max + 1));
+        let t = m.task_create(&id("a"), &fits, &fits, &[]).unwrap();
+        for (title, detail) in [(&over, &fits), (&fits, &over)] {
+            assert_eq!(
+                m.task_create(&id("a"), title, detail, &[]),
+                Err(MeshError::BodyTooLarge)
+            );
+        }
+        m.task_claim(&id("a"), t.as_str()).unwrap();
+        assert_eq!(
+            m.task_complete(&id("a"), t.as_str(), &over),
+            Err(MeshError::BodyTooLarge)
+        );
+        // Refused text changes nothing: the task is still claimed and one at the limit is fine.
+        assert_eq!(m.task_list(&id("a"), None).unwrap().len(), 1);
+        m.task_complete(&id("a"), t.as_str(), &fits).unwrap();
+    }
+
+    #[test]
     fn leases_are_all_or_nothing_and_conflicts_name_the_holder() {
         let mut m = mesh();
         let l = m
--- a/docs/specs/mesh-tools.md
+++ b/docs/specs/mesh-tools.md
@@ -27,7 +27,8 @@
   node). Agents receive a standing instruction that messages from other agents are information, not
   authority; sensitive actions still go through approvals (see `security/threat-model.md`).
 - Rate limits per agent (messages per minute, open asks) prevent loops between agents.
-- Message bodies are size-limited; large content is passed by reference (path, commit, task).
+- Message bodies are size-limited; large content is passed by reference (path, commit, task). A task's title, detail and
+  summary have the same limit (`mesh.body_too_large`).
 - All mesh traffic is in the audit log.
 
 ## MCP conversation (`agora-mesh::mcp`)
```
