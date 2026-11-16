"""Compile the real platform driver, run controls, then the activation regression."""
from pathlib import Path
import subprocess

libs = list(Path("target/debug/deps").glob("libagora_platform*.rlib"))
assert len(libs) == 1, libs
driver = "target/0296-notify-driver"
subprocess.run(["rustc", "--edition=2021", "docs/reviews/0296-notify-driver.rs", "--extern",
                "agora_platform=" + str(libs[0]), "-L", "dependency=target/debug/deps", "-o", driver], check=True)
for command in [
    ["python3", "docs/reviews/0296-title-control.py"],
    ["python3", "docs/reviews/0296-argv-control.py", driver],
    ["dbus-run-session", "--", "/usr/bin/python3", "docs/reviews/0296-dbus-proof.py", driver, "--positive-control"],
]:
    subprocess.run(command, check=True)
result = subprocess.run(["dbus-run-session", "--", "/usr/bin/python3", "docs/reviews/0296-dbus-proof.py", driver], check=False)
raise SystemExit(result.returncode)
