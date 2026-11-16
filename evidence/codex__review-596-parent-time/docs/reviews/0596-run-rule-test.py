"""Run the existing Windows decision helper/test unchanged on Linux.

This executes the pure creation-time rule, not Toolhelp/GetProcessTimes.
"""
from pathlib import Path
import hashlib
import json
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
source = (root / "crates/agora-platform/src/winproc.rs").read_text()


def function(marker):
    start = source.index(marker)
    brace = source.index("{", start)
    depth, end = 1, brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


helper = function("fn started_after(")
test = function("fn a_process_older_than_its_listed_parent_is_not_its_child()")
with tempfile.TemporaryDirectory(prefix="agora-review-596-") as temporary:
    temp = Path(temporary)
    rust = temp / "rule.rs"
    rust.write_text(helper + "\n#[test]\n" + test)
    binary = temp / "rule"
    subprocess.run(["rustc", "--edition=2021", "--test", str(rust), "-o", str(binary)], check=True)
    p = subprocess.run([str(binary)], text=True, capture_output=True)
    assert p.returncode == 0, p.stdout + p.stderr
    assert "1 passed" in p.stdout
    print(json.dumps({"exit": p.returncode, "existing_tests_passed": 1,
                      "helper_sha256": hashlib.sha256(helper.encode()).hexdigest(),
                      "test_sha256": hashlib.sha256(test.encode()).hexdigest(),
                      "source_unchanged": True, "windows_api_execution": False}, indent=2))
