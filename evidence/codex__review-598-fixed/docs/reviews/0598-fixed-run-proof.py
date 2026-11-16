"""Replay the original PR #598 proof, restoring the test source in finally."""
from pathlib import Path
import subprocess

test = Path("crates/agora-hub/tests/server.rs")
original = test.read_bytes()
include = b'''
mod codex_original_home_path_proof {
    use super::*;
    include!("../../../docs/reviews/0598-home-path-proof.rs");
}
'''
assert b"mod codex_original_home_path_proof" not in original
try:
    test.write_bytes(original + include)
    result = subprocess.run([
        "cargo", "test", "-p", "agora-protocol", "-p", "agora-node", "-p", "agora-hub",
        "--test", "server", "review_598_home_username_does_not_reach_nonowners", "--", "--nocapture",
    ], check=False)
finally:
    test.write_bytes(original)
raise SystemExit(result.returncode)
