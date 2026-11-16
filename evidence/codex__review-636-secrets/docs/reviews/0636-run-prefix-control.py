"""Actual node/socket proof plus an optional command-line prefix-pinning control.

Only synthetic scratch files and agora-fake-agent are used. Own worktree only;
do not run concurrently with another build that reads the same source.
"""
from pathlib import Path
import os
import subprocess
import sys

root = Path(__file__).resolve().parents[2]
target = root / "crates/agora-node/tests/codex_review636_temporary.rs"
source = root / "crates/agora-node/src/git.rs"
original = source.read_bytes()
assert not target.exists()
try:
    target.write_bytes((root / "crates/agora-node/tests/control.rs").read_bytes() + b"\n" + (root / "docs/reviews/0636-prefix-control.rs").read_bytes())
    environment = os.environ.copy()
    if "--pin-prefix-control" in sys.argv:
        needle = b'        "--no-textconv",\n'
        start = original.index(b'pub fn diff_with(')
        end = original.index(b'    let tracked = git_capped', start)
        section = original[start:end]
        assert section.count(needle) == 1
        section = section.replace(needle, needle + b'        "--src-prefix=a/",\n        "--dst-prefix=b/",\n')
        source.write_bytes(original[:start] + section + original[end:])
        environment["CODEX636_PINNED_CONTROL"] = "1"
    subprocess.run(["cargo", "test", "-p", "agora-node", "--test", "codex_review636_temporary", "codex_review636_", "--", "--test-threads=1", "--nocapture"], cwd=root, env=environment, check=True)
finally:
    if source.read_bytes() != original:
        source.write_bytes(original)
    target.unlink(missing_ok=True)
