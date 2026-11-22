"""Reproduce the #966 shutdown gap with a disposable machine and real CLI."""

import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("agora", type=Path)
    parser.add_argument("fake_agent", type=Path)
    parser.add_argument("--gap-seconds", type=float, default=20)
    parser.add_argument("--transcript", type=Path)
    return parser.parse_args()


def read_reply(output, request_id, bridge, seconds):
    deadline = time.monotonic() + seconds
    pending = b""
    while time.monotonic() < deadline:
        pending += output.read(65536)
        while b"\n" in pending:
            line, pending = pending.split(b"\n", 1)
            reply = json.loads(line)
            if reply.get("id") == request_id:
                return reply
        if bridge.poll() is not None:
            raise RuntimeError(f"Bridge ended while waiting for request {request_id}")
        time.sleep(0.025)
    raise TimeoutError(f"No reply for request {request_id}")


def write_request(writer, method, request_id=None, params=None):
    request = {"jsonrpc": "2.0", "method": method}
    if request_id is not None:
        request["id"] = request_id
    if params is not None:
        request["params"] = params
    line = (json.dumps(request, separators=(",", ":")) + "\n").encode()
    if os.write(writer, line) != len(line):
        raise OSError("Short write to the FIFO")


def check_gap(args, root):
    agora = args.agora.resolve(strict=True)
    fake_agent = args.fake_agent.resolve(strict=True)
    work = root / "work"
    work.mkdir()
    manifests = root / "manifests"
    manifests.mkdir()
    probe = root / "probe-{pid}.json"
    launch = json.dumps([str(fake_agent), "--probe-directory", str(probe)])
    (manifests / "probe.toml").write_text(
        'kind = "probe"\ndisplay_name = "Probe"\nversion = "1"\nmin_engine = 1\n'
        f"[process]\nlaunch = {launch}\n", encoding="utf-8",
    )
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    (root / "machine.toml").write_text(
        'roles = ["hub", "node"]\nui = "none"\n'
        f'hub_listen = "127.0.0.1:{port}"\nhub_names = ["localhost"]\n'
        f'hub_dir = {json.dumps(str(root / "hub"))}\nnode_projects = []\n',
        encoding="utf-8",
    )
    env = {**os.environ, "AGORA_HOME": str(root), "AGORA_LANG": "en"}

    def run(*command, check=True):
        process = subprocess.run(
            [agora, *command], cwd=work, env=env, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, timeout=90,
        )
        if check and process.returncode != 0:
            raise RuntimeError((command, process.returncode, process.stdout, process.stderr))
        return process.stdout

    bridge = None
    writer = None
    transcript = root / "mesh.stdout"
    request_args = {"name": "workspace_status", "arguments": {}}
    try:
        run("--ui", "none")
        run("agent", "start", "probe", "--cwd", str(work))
        mesh = root / "m" / "agent-1.sock"
        deadline = time.monotonic() + 15
        while not mesh.exists() or not list(root.glob("probe-*.json")):
            if time.monotonic() >= deadline:
                raise TimeoutError("Fake agent or its mesh socket did not appear")
            time.sleep(0.05)

        fifo = root / "mesh.stdin"
        os.mkfifo(fifo)
        writer = os.open(fifo, os.O_RDWR)
        with fifo.open("rb", buffering=0) as inp, transcript.open("wb") as out:
            bridge = subprocess.Popen(
                [agora, "mesh", "stdio", "--socket", mesh],
                stdin=inp, stdout=out, cwd=work, env=env,
            )
        with transcript.open("rb", buffering=0) as replies:
            write_request(writer, "initialize", 1, {"protocolVersion": "2025-03-26"})
            assert "result" in read_reply(replies, 1, bridge, 10)
            write_request(writer, "notifications/initialized")
            write_request(writer, "tools/call", 2, request_args)
            assert "result" in read_reply(replies, 2, bridge, 10)

            down = run("down")
            assert "keeps running" in down, down
            restart_at = time.monotonic() + args.gap_seconds
            write_request(writer, "tools/call", 3, request_args)
            try:
                gap = read_reply(replies, 3, bridge, args.gap_seconds)
            except TimeoutError:
                gap = None
                print("Request 3 was unanswered during the gap.", flush=True)
            else:
                assert gap.get("error", {}).get("data", {}).get("code") == "mesh.unavailable", gap
                print("Request 3 during the gap returned mesh.unavailable.", flush=True)

            time.sleep(max(0, restart_at - time.monotonic()))
            run("--ui", "none")
            deadline = time.monotonic() + 10
            request_id = 4
            while time.monotonic() < deadline:
                write_request(writer, "tools/call", request_id, request_args)
                reply = read_reply(replies, request_id, bridge, 2)
                if "result" in reply:
                    print(f"Request {request_id} after restart returned workspace_status.")
                    if gap is None:
                        raise TimeoutError("Request 3 had no reply during the shutdown gap")
                    return
                request_id += 1
                time.sleep(0.1)
            raise TimeoutError("The original bridge did not reconnect")
    finally:
        try:
            if writer is not None:
                os.close(writer)
            if bridge is not None:
                try:
                    bridge.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    bridge.terminate()
                    bridge.wait(timeout=5)
        finally:
            try:
                if args.transcript is not None and transcript.is_file():
                    shutil.copyfile(transcript, args.transcript)
            finally:
                run("down", "--agents", check=False)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="ag966-", dir="/tmp") as scratch:
        check_gap(arguments(), Path(scratch))
