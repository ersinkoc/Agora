"""Run from the repository root; restore the server test source even on failure."""
from pathlib import Path
import subprocess

test = Path("crates/agora-hub/tests/server.rs")
original = test.read_bytes()
include = b'\ninclude!("../../../docs/reviews/0598-home-path-proof.rs");\n'
assert include not in original
try:
    test.write_bytes(original + include)
    result = subprocess.run([
        "cargo", "test", "-p", "agora-hub", "--test", "server",
        "review_598_home_username_does_not_reach_nonowners", "--", "--nocapture",
    ], check=False)
finally:
    test.write_bytes(original)
raise SystemExit(result.returncode)
