"""Run the actual hub transport regression and restore the existing test file."""
from pathlib import Path
import subprocess

source = Path("crates/agora-hub/tests/server.rs")
original = source.read_bytes()
include = b'\ninclude!("../../../docs/reviews/0629-replay-proof.rs");\n'
assert include not in original
try:
    source.write_bytes(original + include)
    result = subprocess.run(["cargo", "test", "-p", "agora-hub", "--test", "server",
                             "review629_legacy_replay_is_as_safe_as_the_rebuilt_snapshot", "--", "--nocapture"], check=False)
finally:
    source.write_bytes(original)
raise SystemExit(result.returncode)
