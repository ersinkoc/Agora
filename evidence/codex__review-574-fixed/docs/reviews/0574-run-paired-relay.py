"""Review-only paired release measurement; use an isolated worktree of #574."""
from pathlib import Path
import json
import re
import statistics
import subprocess

root = Path(__file__).resolve().parents[2]
lib = root / "crates/agora-hub/src/lib.rs"
baseline = root / "crates/agora-hub/src/review_574_baseline.rs"
assert not baseline.exists()
subprocess.run(["git", "diff", "--exit-code", "--", str(lib)], cwd=root, check=True)
original = lib.read_bytes()
before = subprocess.check_output(["git", "show", "b2fe875:crates/agora-hub/src/router.rs"], cwd=root, text=True).split("#[cfg(test)]")[0]
try:
    baseline.write_text("#![allow(dead_code)]\n" + before)
    lib.write_bytes(original + b"\n#[cfg(test)]\nmod review_574_baseline;\n" + (root / "docs/reviews/0574-paired-relay-bench.rs").read_bytes())
    p = subprocess.run(["cargo", "test", "--release", "-p", "agora-hub", "--lib", "review_574_paired", "--", "--ignored", "--nocapture", "--test-threads=1"], cwd=root, text=True, capture_output=True, timeout=600)
    assert p.returncode == 0, p.stdout + p.stderr
    rows = [{"clients": int(a), "payload_bytes": int(b), "sample": int(c), "before_ns": float(d), "after_ns": float(e)} for a, b, c, d, e in re.findall(r"paired clients=(\d+) payload=(\d+) sample=(\d+) before=([\d.]+) after=([\d.]+)", p.stdout)]
    assert len(rows) == 45
    summary = []
    for clients, size in [(1, 64), (1, 4096), (8, 64), (8, 4096), (32, 4096)]:
        selected = [r for r in rows if (r["clients"], r["payload_bytes"]) == (clients, size)]
        before_ns = statistics.median(r["before_ns"] for r in selected)
        after_ns = statistics.median(r["after_ns"] for r in selected)
        summary.append({"clients": clients, "payload_bytes": size, "before_median_ns": before_ns, "after_median_ns": after_ns, "ratio_percent": round((after_ns / before_ns - 1) * 100, 2)})
    print(json.dumps({"summary": summary, "samples": rows}, indent=2))
finally:
    lib.write_bytes(original)
    baseline.unlink(missing_ok=True)
