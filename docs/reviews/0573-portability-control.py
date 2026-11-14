"""Linux native-browser control for #573 (requires built artifacts and PW_DIR).

Copy current scripts into a temporary repository with spaces/Unicode, select a
Python executable with spaces, and delay hub termination to observe cleanup order.
SOURCE can select a temporary mutant of perf-tree.mjs for negative verification.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

root = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix="agora-573-") as temporary:
    base = Path(temporary)
    repo = base / "repo boşluk"
    (repo / "scripts").mkdir(parents=True)
    (repo / "docs/reviews").mkdir(parents=True)
    source = Path(os.environ.get("SOURCE", root / "scripts/perf-tree.mjs"))
    shutil.copyfile(source, repo / "scripts/perf-tree.mjs")
    shutil.copyfile(root / "scripts/perf-seed-agents.py", repo / "scripts/perf-seed-agents.py")
    shutil.copyfile(root / "docs/reviews/0557-phase-control.mjs", repo / "docs/reviews/0557-phase-control.mjs")
    audit = base / "exit.jsonl"
    python_audit = base / "python-used"
    pids = base / "pids"
    python = base / "python seçimi"
    python.write_text(f"#!{sys.executable}\nimport os, sys\nfrom pathlib import Path\nPath({str(python_audit)!r}).write_text('selected')\nos.execv({sys.executable!r}, [{sys.executable!r}, *sys.argv[1:]])\n")
    python.chmod(0o755)
    binary = str(Path(os.environ.get("AGORA_PERF_BIN", root / "target/debug/agora")).resolve())
    wrapper = base / "agora delayed"
    wrapper.write_text(f"""#!{sys.executable}
import json, os, signal, subprocess, sys, time
from pathlib import Path
if sys.argv[1:3] != ['hub', 'run']:
    os.execv({binary!r}, [{binary!r}, *sys.argv[1:]])
with open({str(pids)!r}, 'a') as f: f.write(str(os.getpid()) + '\\n')
child = subprocess.Popen([{binary!r}, *sys.argv[1:]])
stopping = False
def stop(sig, frame):
    global stopping
    stopping = True
signal.signal(signal.SIGTERM, stop)
while child.poll() is None:
    if stopping:
        time.sleep(0.2)
        before = Path(os.environ['AGORA_HOME']).exists()
        child.terminate()
        try: child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill(); child.wait()
        after = Path(os.environ['AGORA_HOME']).exists()
        with open({str(audit)!r}, 'a') as f:
            f.write(json.dumps({{'home_before_hub_exit': before, 'home_after_hub_exit': after}}) + '\\n')
        sys.exit(0)
    time.sleep(0.02)
sys.exit(child.returncode)
""")
    wrapper.chmod(0o755)
    env = dict(os.environ, PYTHON=str(python), AGORA_PERF_BIN=str(wrapper),
               AGORA_PERF_WEB_DIR=str(Path(os.environ.get("AGORA_PERF_WEB_DIR", root / "apps/web/dist")).resolve()),
               AGORA_PERF_FLEET="10 5 10")
    try:
        with subprocess.Popen(["node", str(repo / "docs/reviews/0557-phase-control.mjs")], env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True) as p:
            try:
                stdout, stderr = p.communicate(timeout=40)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGTERM)
                p.communicate(timeout=6)
                raise
        if p.returncode:
            raise RuntimeError("Native browser control exited " + str(p.returncode) + "\n" + stderr.replace(str(base), "<control>"))
        result = json.loads(stdout)
        assert result["agents"] == 500 and result["reviewControlPassed"]
        assert python_audit.read_text() == "selected", "Python override was not used"
        deadline = time.monotonic() + 3
        while not audit.exists() or len(audit.read_text().splitlines()) < 2:
            assert time.monotonic() < deadline, "Hub exit observation missing"
            time.sleep(0.02)
        exits = [json.loads(line) for line in audit.read_text().splitlines()]
        assert len(exits) == 2 and all(all(e.values()) for e in exits), "Temporary home removed before hub exit"
        print(json.dumps({"repository_path_has_spaces_and_unicode": True, "python_override_used": True,
                          "hub_exit_observations": exits, "benchmark": result}, indent=2))
    finally:
        # On an assertion/timeout, stop only this control's own still-running wrappers.
        for pid in pids.read_text().splitlines() if pids.exists() else []:
            try:
                if str(wrapper).encode() in Path(f"/proc/{pid}/cmdline").read_bytes():
                    os.kill(int(pid), signal.SIGTERM)
            except (ProcessLookupError, FileNotFoundError):
                pass
