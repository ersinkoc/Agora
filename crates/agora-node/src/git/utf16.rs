//! Changes of UTF-16 text files as text (#940). Windows PowerShell 5.1's `>` writes UTF-16 LE with a byte-order mark; git sees the
//! NUL bytes and prints `Binary files ... differ`, and does so for any repository unless a `textconv` program is set up, which the
//! node never runs (`--no-textconv`: a repository's configuration must not start programs). A UTF-16 file with no zero byte at
//! all (one line of characters none of which is below U+0100, with no line break) is text to git, shown as the raw bytes. So for a
//! section git called binary, or whose first line starts with the bytes of a mark (`FF FE`/`FE FF`, two replacement characters
//! once git's output is read as UTF-8), where a side is UTF-16 with a mark ([`agora_core::text::utf16_text`]), the node reads both
//! sides itself and writes the section as git would for that text: decoded for showing, the bytes on disk never changed or
//! written back. Everything else git printed, a real binary file among it (no mark, or not valid UTF-16), is left exactly as it
//! was.

use std::path::Path;

use super::{quote_path, section_paths, MAX_UNTRACKED, MAX_UNTRACKED_BYTES};

/// Lines of context around a change, as git prints by default.
const CONTEXT: usize = 3;

/// The most edits the line diff looks for before it shows the changed middle as all removed and all added (still a correct diff,
/// only a longer one). Keeps the time and memory of one file small: both are at most proportional to this times the line count.
const MAX_EDITS: usize = 500;

/// Where git's output, read as UTF-8, shows a byte-order mark of UTF-16 at the start of a hunk line.
const MARK: &str = "\u{fffd}\u{fffd}";

/// `diff` (git's output for the tracked files) with each section of a UTF-16 text file written as text. At most
/// [`MAX_UNTRACKED`] sections are read, each side at most [`MAX_UNTRACKED_BYTES`], as for new files; a section past those stays
/// as git printed it. Unless `show_secret` (the owner's policy, ADR-0080), the workspace's file is read with the secret check of
/// `files.read`, on where the opened file really is: one that leads to a secret file is not read, and its section stays git's.
pub(super) fn decode_sections(worktree: &Path, diff: String, show_secret: bool) -> String {
    if !diff.starts_with("Binary files ")
        && !diff.contains("\nBinary files ")
        && !diff.contains(MARK)
    {
        return diff;
    }
    let lines: Vec<&str> = diff.split_inclusive('\n').collect();
    let mut out = String::with_capacity(diff.len());
    let mut tries = 0;
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("diff --git ") {
            out.push_str(lines[i]);
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < lines.len() && !lines[end].starts_with("diff --git ") {
            end += 1;
        }
        let section = &lines[i..end];
        i = end;
        let text = match header_end(section) {
            Some(at) if tries < MAX_UNTRACKED => {
                tries += 1;
                as_text(worktree, section, at, show_secret)
            }
            _ => None,
        };
        match text {
            Some(t) => out.push_str(&t),
            None => section.iter().for_each(|l| out.push_str(l)),
        }
    }
    out
}

/// For a section that may be a UTF-16 file, where git's header ends (its `Binary files` line, or the `---` line of a text section
/// with a hunk line that starts with [`MARK`]); `None` for any other section. Only a guess: the bytes of both sides decide.
fn header_end(section: &[&str]) -> Option<usize> {
    if let Some(at) = section.iter().position(|l| l.starts_with("Binary files ")) {
        return Some(at);
    }
    let hunk = section.iter().position(|l| l.starts_with("@@ "))?;
    let marked = section[hunk..]
        .iter()
        .any(|l| l.get(1..).is_some_and(|c| c.starts_with(MARK)));
    if !marked {
        return None;
    }
    section[..hunk].iter().position(|l| l.starts_with("--- "))
}

/// One section as text, or `None` to keep it: neither side is UTF-16 text, a side is not text at all, a side cannot be read (too
/// large, gone, not a plain file, secret), or the two texts are the same (only the bytes differ, as in a change of encoding, which
/// git's own line says better than an empty text diff would). `header_end`: git's lines before it are kept.
fn as_text(
    worktree: &Path,
    section: &[&str],
    header_end: usize,
    show_secret: bool,
) -> Option<String> {
    let has = |p: &str| section.iter().any(|l| l.starts_with(p));
    let (added, deleted) = (has("new file mode "), has("deleted file mode "));
    let (_, new) = section_paths(section);
    let old = section
        .iter()
        .find_map(|l| {
            l.strip_prefix("rename from ")
                .or_else(|| l.strip_prefix("copy from "))
        })
        .map_or_else(|| new.clone(), |p| super::unquote(p.trim_end()));
    let old_bytes = if added {
        None
    } else {
        Some(at_head(worktree, &old)?)
    };
    let new_bytes = if deleted {
        None
    } else {
        // A secret name's section was cut before this; the read checks again where the opened file really is (a folder swapped
        // for a link to `.ssh` after git ran), as `files.read` does.
        let files = crate::files::Files::open(worktree, MAX_UNTRACKED_BYTES)
            .ok()?
            .with_secret_shown(show_secret);
        Some(files.read(&new).ok()?)
    };
    let mut utf16 = false;
    let mut side = |bytes: &Option<Vec<u8>>| -> Option<String> {
        let Some(b) = bytes else {
            return Some(String::new());
        };
        if let Some(t) = agora_core::text::utf16_text(b) {
            utf16 = true;
            return Some(t);
        }
        // git's rule: a NUL in the first 8000 bytes makes a side binary.
        (!b[..b.len().min(8000)].contains(&0)).then(|| String::from_utf8_lossy(b).into_owned())
    };
    let (a, b) = (side(&old_bytes)?, side(&new_bytes)?);
    if !utf16 || a == b {
        return None;
    }
    let mut out = String::new();
    section[..header_end].iter().for_each(|l| out.push_str(l));
    let name = |prefix: &str, path: &str, absent: bool| {
        if absent {
            return "/dev/null".to_string();
        }
        let q = quote_path(&format!("{prefix}/{path}"));
        // git ends a name holding a space with a tab on these two lines.
        let tab = if q.contains(' ') { "\t" } else { "" };
        format!("{q}{tab}")
    };
    out.push_str(&format!(
        "--- {}\n+++ {}\n",
        name("a", &old, added),
        name("b", &new, deleted)
    ));
    out.push_str(&unified(&a, &b));
    Some(out)
}

/// The file `path` (named from the worktree, as `--relative` names it) as HEAD has it, at most [`MAX_UNTRACKED_BYTES`].
fn at_head(worktree: &Path, path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let spec = format!("HEAD:./{path}");
    let mut child = super::hardened(worktree)
        .args(["cat-file", "blob", &spec])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut buf = Vec::new();
    let read = match child.stdout.take() {
        Some(out) => out
            .take(MAX_UNTRACKED_BYTES + 1)
            .read_to_end(&mut buf)
            .is_ok(),
        None => false,
    };
    if !read || buf.len() as u64 > MAX_UNTRACKED_BYTES {
        // Past the cap git may still be writing; killing it is how it stops.
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    // A blob under the cap was read to its end: git's own exit says whether that is the whole file.
    child.wait().ok()?.success().then_some(buf)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    Keep,
    Remove,
    Add,
}

/// The hunks of a unified diff from `a` to `b` (no header), as git prints them: three lines of context, hunks closer than twice
/// that joined, a count of 1 left out of a range, and `\ No newline at end of file` after a last line without one.
fn unified(a: &str, b: &str) -> String {
    let (al, bl): (Vec<&str>, Vec<&str>) = (
        a.split_inclusive('\n').collect(),
        b.split_inclusive('\n').collect(),
    );
    let edits = edits(&al, &bl);
    // Where each edit starts in `a` and in `b`.
    let mut at = Vec::with_capacity(edits.len() + 1);
    let (mut x, mut y) = (0usize, 0usize);
    for e in &edits {
        at.push((x, y));
        match e {
            Edit::Keep => {
                x += 1;
                y += 1;
            }
            Edit::Remove => x += 1,
            Edit::Add => y += 1,
        }
    }
    at.push((x, y));
    let changes: Vec<usize> = (0..edits.len())
        .filter(|&i| edits[i] != Edit::Keep)
        .collect();
    let mut out = String::new();
    let mut c = 0;
    while c < changes.len() {
        let first = changes[c];
        let mut last = first;
        c += 1;
        while c < changes.len() && changes[c] - last - 1 <= 2 * CONTEXT {
            last = changes[c];
            c += 1;
        }
        let (start, end) = (
            first.saturating_sub(CONTEXT),
            (last + 1 + CONTEXT).min(edits.len()),
        );
        let ((x0, y0), (x1, y1)) = (at[start], at[end]);
        let range = |from: usize, n: usize| match n {
            0 => format!("{from},0"),
            1 => format!("{}", from + 1),
            n => format!("{},{n}", from + 1),
        };
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            range(x0, x1 - x0),
            range(y0, y1 - y0)
        ));
        for (i, e) in edits.iter().enumerate().take(end).skip(start) {
            let (mark, line) = match e {
                Edit::Keep => (' ', al[at[i].0]),
                Edit::Remove => ('-', al[at[i].0]),
                Edit::Add => ('+', bl[at[i].1]),
            };
            out.push(mark);
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    out
}

/// A shortest edit script from `a` to `b` (Myers' algorithm on what is left between the common start and end), or, past
/// [`MAX_EDITS`], that middle all removed then all added.
fn edits(a: &[&str], b: &[&str]) -> Vec<Edit> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (ar, br) = (&a[pre..], &b[pre..]);
    let suf = ar
        .iter()
        .rev()
        .zip(br.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&ar[..ar.len() - suf], &br[..br.len() - suf]);
    let mut out = vec![Edit::Keep; pre];
    match myers(am, bm) {
        Some(mid) => out.extend(mid),
        None => {
            out.extend(std::iter::repeat_n(Edit::Remove, am.len()));
            out.extend(std::iter::repeat_n(Edit::Add, bm.len()));
        }
    }
    out.extend(std::iter::repeat_n(Edit::Keep, suf));
    out
}

/// Myers' greedy shortest edit script, `None` when it needs more than [`MAX_EDITS`] edits. Each step's furthest points are kept
/// only for the diagonals that step can reach, so memory grows with the square of the edits, not with the file.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<Edit>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = a.len() + b.len();
    let off = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    // trace[d]: the furthest x on diagonals -(d-1)..=(d-1) before step d.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let idx = |k: isize| (k + off) as usize;
    for d in 0..=max.min(MAX_EDITS) as isize {
        trace.push(if d == 0 {
            Vec::new()
        } else {
            v[idx(-(d - 1))..=idx(d - 1)].to_vec()
        });
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) {
                v[idx(k + 1)]
            } else {
                v[idx(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx(k)] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, n, m));
            }
            k += 2;
        }
    }
    None
}

fn backtrack(trace: &[Vec<isize>], n: isize, m: isize) -> Vec<Edit> {
    let mut out = Vec::new();
    let (mut x, mut y) = (n, m);
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let (px, py) = if d == 0 {
            (0, 0)
        } else {
            let get = |k: isize| v[(k + d - 1) as usize];
            let k = x - y;
            let pk = if k == -d || (k != d && get(k - 1) < get(k + 1)) {
                k + 1
            } else {
                k - 1
            };
            let px = get(pk);
            (px, px - pk)
        };
        while x > px && y > py {
            out.push(Edit::Keep);
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            out.push(if x == px { Edit::Add } else { Edit::Remove });
        }
        (x, y) = (px, py);
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies `edits` to `a`, which must give `b`.
    fn apply(a: &[&str], b: &[&str], edits: &[Edit]) -> Vec<String> {
        let (mut x, mut y, mut out) = (0, 0, Vec::new());
        for e in edits {
            match e {
                Edit::Keep => {
                    assert_eq!(a[x], b[y]);
                    out.push(a[x].to_string());
                    x += 1;
                    y += 1;
                }
                Edit::Remove => x += 1,
                Edit::Add => {
                    out.push(b[y].to_string());
                    y += 1;
                }
            }
        }
        assert_eq!((x, y), (a.len(), b.len()));
        out
    }

    #[test]
    fn the_edit_script_turns_one_side_into_the_other_and_is_shortest() {
        let cases: [(&str, &str, usize); 6] = [
            ("", "", 0),
            ("a\nb\nc\n", "a\nb\nc\n", 0),
            ("a\nb\nc\na\nb\nb\na\n", "c\nb\na\nb\na\nc\n", 5),
            ("", "x\ny\n", 2),
            ("x\ny\n", "", 2),
            ("1\n2\n3\n4\n", "1\n3\n4\n5\n", 2),
        ];
        for (a, b, want) in cases {
            let (al, bl): (Vec<&str>, Vec<&str>) = (
                a.split_inclusive('\n').collect(),
                b.split_inclusive('\n').collect(),
            );
            let e = edits(&al, &bl);
            assert_eq!(apply(&al, &bl, &e), bl, "{a:?} -> {b:?}");
            assert_eq!(
                e.iter().filter(|e| **e != Edit::Keep).count(),
                want,
                "{a:?} -> {b:?}"
            );
        }
    }

    #[test]
    fn past_the_edit_limit_the_middle_is_replaced_whole() {
        let a: Vec<String> = (0..2000).map(|n| format!("{n}\n")).collect();
        let b: Vec<String> = (0..2000).map(|n| format!("{}\n", n * 7 + 1)).collect();
        let (al, bl): (Vec<&str>, Vec<&str>) = (
            a.iter().map(String::as_str).collect(),
            b.iter().map(String::as_str).collect(),
        );
        let e = edits(&al, &bl);
        assert_eq!(apply(&al, &bl, &e), bl);
    }

    #[test]
    fn hunks_are_written_as_git_writes_them() {
        assert_eq!(unified("", "a\n"), "@@ -0,0 +1 @@\n+a\n");
        assert_eq!(unified("a\nb", "a\nc"), "@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n");
        assert_eq!(
            unified("a\n", "a"),
            "@@ -1 +1 @@\n-a\n+a\n\\ No newline at end of file\n"
        );
        // Changes with six lines between them share a hunk; with seven they do not.
        let lines = |n: usize, changed: [usize; 2]| {
            (1..=n)
                .map(|i| {
                    if changed.contains(&i) {
                        format!("x{i}\n")
                    } else {
                        format!("{i}\n")
                    }
                })
                .collect::<String>()
        };
        let one = unified(&lines(12, [0, 0]), &lines(12, [2, 9]));
        assert_eq!(one.matches("@@ -").count(), 1, "{one}");
        let two = unified(&lines(12, [0, 0]), &lines(12, [2, 10]));
        assert_eq!(two.matches("@@ -").count(), 2, "{two}");
        assert!(two.starts_with("@@ -1,5 +1,5 @@\n"), "{two}");
        assert!(two.contains("@@ -7,6 +7,6 @@\n"), "{two}");
    }
}
