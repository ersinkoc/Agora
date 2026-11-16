"""Compile the real libraries' proof driver; bound even the old reflow hang."""
from pathlib import Path
import subprocess
import tempfile

command = ["rustc", "--edition=2021", "docs/reviews/0620-narrow-driver.rs", "-L", "dependency=target/debug/deps", "-o", "target/0620-narrow-driver"]
for name in ["vt", "node", "platform"]:
    libs = list(Path("target/debug/deps").glob("libagora_"+name+"-*.rlib"))
    assert len(libs) == 1, libs
    command += ["--extern", "agora_"+name+"="+str(libs[0])]
subprocess.run(command, check=True)
for mode in ["feed-zero", "feed-one", "reflow"]:
    subprocess.run(["target/0620-narrow-driver", mode], check=True, timeout=6)
with tempfile.TemporaryDirectory(prefix="0620 native ", dir="target") as temporary:
    subprocess.run(["target/0620-narrow-driver", "node", str(Path(temporary).resolve()/"size.txt")], check=True, timeout=20)
