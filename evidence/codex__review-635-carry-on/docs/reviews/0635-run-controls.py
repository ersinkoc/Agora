"""Run independent controls using the PR's real-node/fake-agent fixture.

Writes one temporary integration target, restores it in finally, and optionally
removes only the working-state gate as a negative control in this own worktree.
Never runs a real coding agent or opens a user's ledger/configuration.
"""
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[2]
target = root / "crates/agora-node/tests/codex_review635_temporary.rs"
ledger = root / "crates/agora-node/src/ledger.rs"
original = ledger.read_bytes()
assert not target.exists()
try:
    harness = (root / "crates/agora-node/tests/session_resume.rs").read_bytes()
    fragment = (root / "docs/reviews/0635-independent-controls.rs").read_bytes()
    target.write_bytes(harness + b"\n" + fragment)
    if "--negative" in sys.argv:
        before = b'e.last_state == "working" && e.continue_sent_ms < e.interrupted_at_ms'
        after = b'e.continue_sent_ms < e.interrupted_at_ms'
        assert original.count(before) == 1
        ledger.write_bytes(original.replace(before, after))
        selected = "the_carry_on_sentence_is_never_sent_unless"
    else:
        selected = "codex_review635_"
    result = subprocess.run([
        "cargo", "test", "-p", "agora-node", "--test", "codex_review635_temporary",
        selected, "--", "--test-threads=1", "--nocapture",
    ], cwd=root, capture_output=True, text=True)
    print(result.stdout)
    print(result.stderr)
    if "--negative" in sys.argv:
        assert result.returncode != 0, "removing the safety gate did not fail the test"
        assert "blocked: the sentence was sent" in result.stdout + result.stderr
        print("Expected failure observed with only the working-state gate removed.")
    else:
        assert result.returncode == 0, result.returncode
finally:
    if ledger.read_bytes() != original:
        ledger.write_bytes(original)
    target.unlink(missing_ok=True)
