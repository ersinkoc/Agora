"""Run only in an isolated worktree of #562 at a87f339; restores product files."""
from pathlib import Path
import json
import re
import subprocess

root = Path(__file__).resolve().parents[2]
paths = [root / "crates/agora-hub/src" / name for name in ["hub.rs", "signin.rs", "web.rs"]]
paths += [root / "crates/agora-hub/tests/server.rs"]
hub, signin, web, server = paths
subprocess.run(["git", "diff", "--exit-code", "--", *map(str, paths)], cwd=root, check=True)
original = {p: p.read_bytes() for p in paths}
results = []


def run(name, args, expected, marker=None):
    p = subprocess.run(args, cwd=root, text=True, capture_output=True, timeout=120)
    found = marker is None or marker in p.stdout
    results.append({"name": name, "exit": p.returncode, "expected_exit": expected, "expected_marker_found": found})
    assert p.returncode == expected, (name, p.stdout, p.stderr)
    assert found, (name, p.stdout, p.stderr)


try:
    for target, proof in [(hub, "0562-auth-work-proof.rs"), (server, "0562-http-proof.rs")]:
        target.write_bytes(original[target] + b"\n" + (root / "docs/reviews" / proof).read_bytes())
    auth = ["cargo", "test", "-p", "agora-hub", "--lib", "review_562_auth_before_work"]
    http = ["cargo", "test", "-p", "agora-hub", "--test", "server", "review_562_disabled"]
    run("auth-fixed", auth, 0)
    run("http-fixed", http, 0)
    old = subprocess.check_output(["git", "show", "f8cc09c:crates/agora-hub/src/signin.rs"], cwd=root, text=True)

    def method(text):
        return re.search(r"    fn metrics\(&self, bearer: Option<&str>\).*?(?=    fn info\()", text, re.S).group(0)

    current = original[signin].decode()
    signin.write_text(current.replace(method(current), method(old)))
    run("auth-original", auth, 101, "rejected scrape reached blocked metrics work")
    signin.write_bytes(original[signin])
    marker = b"        && sessions.is_some_and(|api| api.metrics_on())\n"
    assert original[web].count(marker) == 1
    web.write_bytes(original[web].replace(marker, b""))
    run("http-original", http, 101, "GET /api/metrics")
finally:
    for p, data in original.items():
        p.write_bytes(data)

print(json.dumps(results, indent=2))
