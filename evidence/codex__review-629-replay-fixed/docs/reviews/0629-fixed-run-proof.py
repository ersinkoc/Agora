"""Run review-only transport/parity tests against the actual restored crate."""
from pathlib import Path
import subprocess
import sys

files = {
    Path("crates/agora-hub/tests/server.rs"): b'\ninclude!("../../../docs/reviews/0629-fixed-transport-proof.rs");\n',
    Path("crates/agora-hub/src/lib.rs"): b'\n#[cfg(test)]\n#[allow(clippy::unwrap_used)]\n#[path = "../../../docs/reviews/0629-fixed-parity-proof.rs"]\nmod codex_review629_fixed;\n',
}
control = "--no-barrier-control" in sys.argv
if control:
    files[Path("crates/agora-hub/src/hub.rs")] = b""
original = {p: p.read_bytes() for p in files}
try:
    for p, include in files.items():
        assert not include or include not in original[p]
        p.write_bytes(original[p] + include)
    if control:
        hub = Path("crates/agora-hub/src/hub.rs")
        guard = b"(g.view.distrusted_until == 0 || n > g.view.distrusted_until)"
        assert original[hub].count(guard) == 1
        hub.write_bytes(original[hub].replace(guard, b"true"))
    selector = "review629_fixed_checkpoints_heal_and_safe_tail_replays" if control else "review629_fixed_"
    result = subprocess.run(["cargo", "test", "-p", "agora-hub", selector, "--", "--nocapture"], check=False)
finally:
    for p, content in original.items():
        p.write_bytes(content)
raise SystemExit(result.returncode)
