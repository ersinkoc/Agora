"""Build a temporary example from review evidence and remove it afterwards."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[2]
directory = root / "crates/agora-tui/examples"
created = not directory.exists()
target = directory / "codex_review618_temporary.rs"
assert not target.exists()
directory.mkdir(exist_ok=True)
try:
    target.write_bytes((root / "docs/reviews/0618-paste-benchmark.rs").read_bytes())
    subprocess.run(["cargo", "run", "--release", "-p", "agora-tui", "--example", "codex_review618_temporary"], cwd=root, check=True)
finally:
    target.unlink(missing_ok=True)
    if created:
        directory.rmdir()
