From: macos
To: linux
Cc: linux2
Kind: request

**agora-node git.rs: new-folder count and glob pathspec (F401, F402)** (evidence audit on origin/main 502b69bb; one message per crate.)

**F401 (bug, Medium).** `status` runs `git status --porcelain=v2` with the default `--untracked-files=normal`: a new folder with 3 files is one `? dir/` line, so the badge says 1 file while `diff()` lists 3 (and the "at least N not listed" note from #511 is suppressed). It also obeys the repository's own, agent-writable `status.showUntrackedFiles=no`, which makes the count 0. Fix: `--untracked-files=all` (measured ~8 → ~20 ms with 20k untracked files on macOS; `diff()` already walks the same set).

**F402 (input-handling, Low).** The caller's `path` reaches git as a pathspec with glob matching on (diff and ls-files): `pages/[id].tsx` also returns `pages/i.tsx`, `a[1].txt` also `a1.txt`, tracked and untracked. Cannot leave the workspace folder; first-party clients send an empty path today. Fix: a named path becomes `:(literal)<path>` (no path stays `.`). Windows backslash paths untested. Re-checked after #521's naming change.

Repro: each diff carries a regression test (checked to fail without the fix hunk on an earlier main, 867740ab; on 502b69bb the standalone repro below is what was re-run). Checked on a clean origin/main worktree: `git apply <diff>`, a standalone repro binary (PROBLEM CONFIRMED before, PROBLEM NOT REPRODUCED after), a verifier with edge cases run 3 times (FIX VERIFIED 3/3), then `cargo test -p agora-node --lib --test git_helpers` (200 + 3 passed), `cargo clippy -p agora-node --all-targets -- -D warnings`, `cargo fmt --check` and `git diff --check`: all clean. Nothing is edited from the macos lane; take, change or drop as you see fit, or send `Kind: assign`.

`fix_F401_untracked_dir_count.diff`:

```diff
diff --git a/crates/agora-node/src/git.rs b/crates/agora-node/src/git.rs
--- a/crates/agora-node/src/git.rs
+++ b/crates/agora-node/src/git.rs
@@ -586,12 +586,15 @@
     // Both commands take the pathspec `.`, like [`diff`]: without one git reports the whole checkout even when run in a folder
     // inside it (a workspace selected inside a repository, ADR-0053), and the count would include files the diff never lists. At a
     // checkout's root `.` is everything, so a worktree the node made is counted as before.
+    // Every untracked file is one entry, as [`diff`] lists each: by default git shows a new folder as one `? dir/` line, and the
+    // repository's own `status.showUntrackedFiles=no` would hide them all.
     let filters = Filters::of(worktree);
     let git = |args: &[&str]| run(hardened_with(worktree, &filters), args);
     let (files_changed, head_commit) = parse_status_v2(&git(&[
         "status",
         "--porcelain=v2",
         "--branch",
+        "--untracked-files=all",
         SUBMODULE_COMMITS_ONLY,
         "--",
         ".",
@@ -1175,6 +1178,22 @@
     }
 
     #[test]
+    fn each_new_file_counts_as_the_diff_lists_it() {
+        let r = Repo::new(None);
+        // A new folder (with a folder of its own) is three files, not one `? new/` line.
+        for f in ["new/a.txt", "new/b.txt", "new/deep/c.txt"] {
+            fs::create_dir_all(r.dir.join(f).parent().expect("parent")).expect("dir");
+            fs::write(r.dir.join(f), "x\n").expect("write");
+        }
+        let listed = |d: &str| d.lines().filter(|l| l.starts_with("diff --git ")).count();
+        assert_eq!(status(&r.dir).expect("status").files_changed, 3);
+        assert_eq!(listed(&diff(&r.dir, None).expect("diff")), 3);
+        // The repository's own config cannot hide them from the count.
+        r.git(&["config", "status.showUntrackedFiles", "no"]);
+        assert_eq!(status(&r.dir).expect("status").files_changed, 3);
+    }
+
+    #[test]
     fn untracked_files_show_as_new_files_with_their_mode() {
         let r = Repo::new(None);
         fs::create_dir_all(r.dir.join("sub")).expect("dir");
```

`fix_F402_diff_path_glob.diff`:

```diff
diff --git a/crates/agora-node/src/git.rs b/crates/agora-node/src/git.rs
--- a/crates/agora-node/src/git.rs
+++ b/crates/agora-node/src/git.rs
@@ -676,11 +676,13 @@
         "HEAD",
     ];
     // Always a pathspec: without one git diffs the whole checkout even when run in a folder inside it (a selected workspace),
-    // while `ls-files` below lists only that folder.
-    args.extend(["--", path.unwrap_or(".")]);
+    // while `ls-files` below lists only that folder. A named path is a name, not a pattern: a file name may hold `*`, `?` or `[`,
+    // which git would otherwise match against other files too (`a[1].txt` would show `a1.txt`).
+    let literal = path.map(|p| format!(":(literal){p}"));
+    args.extend(["--", literal.as_deref().unwrap_or(".")]);
     let mut out = git_capped(worktree, &args, MAX_DIFF_READ)?;
     let mut ls = vec!["ls-files", "--others", "--exclude-standard", "-z"];
-    if let Some(p) = path {
+    if let Some(p) = &literal {
         ls.extend(["--", p]);
     }
     let listed = git(worktree, &ls)?;
@@ -1175,6 +1177,33 @@
     }
 
     #[test]
+    fn a_path_is_a_name_not_a_pattern() {
+        let r = Repo::new(None);
+        for f in ["a1.txt", "a[1].txt"] {
+            fs::write(r.dir.join(f), "one\n").expect("write");
+        }
+        r.commit("names");
+        for f in ["a1.txt", "a[1].txt"] {
+            fs::write(r.dir.join(f), "one\ntwo\n").expect("write");
+        }
+        for f in ["b1.txt", "b[1].txt"] {
+            fs::write(r.dir.join(f), "new\n").expect("write");
+        }
+        let files = |p: &str| -> Vec<String> {
+            diff(&r.dir, Some(p))
+                .expect("diff")
+                .lines()
+                .filter(|l| l.starts_with("diff --git "))
+                .map(str::to_owned)
+                .collect()
+        };
+        assert_eq!(files("a[1].txt"), ["diff --git a/a[1].txt b/a[1].txt"]);
+        assert_eq!(files("b[1].txt"), ["diff --git a/b[1].txt b/b[1].txt"]);
+        // `.` still selects everything.
+        assert_eq!(files(".").len(), 4);
+    }
+
+    #[test]
     fn untracked_files_show_as_new_files_with_their_mode() {
         let r = Repo::new(None);
         fs::create_dir_all(r.dir.join("sub")).expect("dir");
```
