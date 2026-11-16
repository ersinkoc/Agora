"""Exercise the real platform function with a private executable notifier shim."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

driver = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="0296 argv ü ", dir="target") as temporary:
    work = Path(temporary).resolve()
    notifier = work / "notify-send"
    record = work / "argv.json"
    marker = work / "must-not-exist"
    notifier.write_text("#!/usr/bin/python3\nimport json,os,sys\n"
                        "with open(os.environ['REVIEW_NOTIFY_RECORD'],'w') as f: json.dump(sys.argv[1:],f)\n"
                        "sys.exit(int(os.environ.get('REVIEW_NOTIFY_EXIT','0')))\n")
    notifier.chmod(0o700)
    env = dict(os.environ, PATH=str(work), REVIEW_NOTIFY_RECORD=str(record))
    title = '--help $(touch "' + str(marker) + '") `id` Türkçe'
    body = "' ; echo untrusted & <b>quoted</b> \"body\"\nnext line"
    def call(environment):
        return subprocess.run([driver, title, body], env=environment, text=True,
                              capture_output=True, timeout=5, check=True).stdout.strip()
    assert call(env) == "ok"
    assert json.loads(record.read_text()) == ["--app-name=" + title, "--", title, body]
    assert not marker.exists()
    assert call(dict(env, REVIEW_NOTIFY_EXIT="17")) == "error:Other"
    notifier.chmod(0o600)
    assert call(env) == "error:NotFound"
    notifier.chmod(0o700)
    assert call(dict(env, PATH=".")) == "error:NotFound"
    assert call({k:v for k,v in env.items() if k != "PATH"}) == "error:NotFound"
print("5 real platform calls: literal argv/injection/option delimiter, nonzero status, nonexecutable notifier, relative PATH and missing PATH pass.")
