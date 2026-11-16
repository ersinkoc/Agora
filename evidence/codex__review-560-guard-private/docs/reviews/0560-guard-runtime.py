"""Linux review of #560: real read-only flock holder, SIGSTOP, isolated home.

Run after cargo build -p agora: python3 docs/reviews/0560-guard-runtime.py
No product source is changed. The known down error-reporting follow-up is observed,
not treated as proof that down handles the error correctly.
"""
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time

binary = Path(os.environ.get("AGORA_BIN", Path(__file__).resolve().parents[2] / "target/debug/agora"))
holder_code = """import fcntl, os, signal, sys
with open(sys.argv[1], 'rb') as f:
    fcntl.flock(f, fcntl.LOCK_EX)
    print('locked', flush=True)
    os.kill(os.getpid(), signal.SIGSTOP)
    signal.pause()
"""
report = {"reviewed_head": "ad6e878ef78ee0d6e0b4b2ee0e6971b8a456b7e3"}
with tempfile.TemporaryDirectory(prefix="agora-review-560-") as temporary:
    home = Path(temporary)
    run = home / "run"
    run.mkdir(mode=0o775)
    run.chmod(0o775)
    guard = run / "hub.lock.guard"
    guard.write_text("retained\n")
    guard.chmod(0o664)
    (home / "machine.toml").write_text('roles = ["hub"]\nui = "none"\nhub_listen = "127.0.0.1:0"\nhub_dir = ' + json.dumps(str(home / "hub")) + '\n')
    env = dict(os.environ, AGORA_HOME=str(home), AGORA_LANG="en")

    def command(args):
        started = time.monotonic()
        p = subprocess.run([str(binary), *args], env=env, text=True, capture_output=True, timeout=16)
        return {"seconds": round(time.monotonic() - started, 3), "exit": p.returncode,
                "stdout": p.stdout.replace(str(home), "<isolated-home>"),
                "stderr": p.stderr.replace(str(home), "<isolated-home>")}

    holder = subprocess.Popen([sys.executable, "-c", holder_code, str(guard)], stdout=subprocess.PIPE, text=True)
    try:
        assert holder.stdout.readline().strip() == "locked"
        deadline = time.monotonic() + 3
        while Path(f"/proc/{holder.pid}/stat").read_text().split(") ")[1][0] != "T":
            assert time.monotonic() < deadline
            time.sleep(0.01)
        report["holder"] = "real read-only flock, confirmed SIGSTOP state T"
        p = command(["supervise", "hub", "--home", str(home)])
        assert p["exit"] == 1 and 9.8 <= p["seconds"] < 15
        assert "10 seconds" in p["stderr"]
        assert stat.S_IMODE(run.stat().st_mode) == 0o700
        assert stat.S_IMODE(guard.stat().st_mode) == 0o600
        assert guard.read_text() == "retained\n"
        assert not (run / "hub.lock").exists()
        report["supervise_stopped_holder"] = p
        report["existing_permissions_tightened"] = {"run": "0700", "guard": "0600", "contents_unchanged": True}

        # No real role is signalled: this impossible PID is a stale lock fixture.
        lock = run / "hub.lock"
        lock.write_text("4294967295 1 0.3.11\n")
        p = command(["down"])
        report["down_stale_lock_stopped_holder"] = dict(p, stale_lock_left=lock.exists(), reports_busy="10 seconds" in p["stderr"])
        assert 9.8 <= p["seconds"] < 15
        # This head silently discards the error; the existing Windows follow-up owns it.
        assert p["exit"] == 0 and lock.exists() and not p["stderr"]
    finally:
        holder.kill()  # SIGKILL also releases the OS lock of a stopped process.
        holder.wait(timeout=3)
        holder.stdout.close()

    guard.unlink()
    target = home / "untouched"
    target.write_text("keep\n")
    guard.symlink_to(target)
    p = command(["supervise", "hub", "--home", str(home)])
    assert p["exit"] == 1 and p["seconds"] < 3 and target.read_text() == "keep\n"
    report["symlink_guard_refused"] = dict(p, target_unchanged=True)
    guard.unlink()
    os.mkfifo(guard)
    p = command(["supervise", "hub", "--home", str(home)])
    assert p["exit"] == 1 and p["seconds"] < 3
    report["fifo_guard_refused_without_waiting"] = p

print(json.dumps(report, indent=2))
