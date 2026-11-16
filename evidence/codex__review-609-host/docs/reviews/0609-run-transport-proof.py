"""Include review-only tests in the actual crate; always restore its source."""
from pathlib import Path
import subprocess
import sys

source = Path("crates/agora-desktop-tunnel/src/lib.rs")
original = source.read_bytes()
include = b'\n#[cfg(test)]\n#[allow(clippy::unwrap_used)]\n#[path = "../../../docs/reviews/0609-transport-proof.rs"]\nmod codex_review_609;\n'
assert b"mod codex_review_609" not in original
try:
    instrumented = original + include
    selector = "review609_"
    if "--bypass-guard-control" in sys.argv:
        # Deliberate negative control against a fake loopback hub only. The
        # reviewed source is restored in finally; this is never an implementation.
        guard = b"if !addressed_to_tunnel(&first[..head_len], local.port()) {"
        assert instrumented.count(guard) == 1
        instrumented = instrumented.replace(guard, b"if false && !addressed_to_tunnel(&first[..head_len], local.port()) {")
        selector = "review609_rejections_never_dial_and_valid_control_still_works"
    source.write_bytes(instrumented)
    result = subprocess.run(["cargo", "test", "-p", "agora-desktop-tunnel", selector, "--", "--nocapture"], check=False)
finally:
    source.write_bytes(original)
raise SystemExit(result.returncode)
