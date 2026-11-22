#!/usr/bin/env python3
"""A program for the terminal repaint checks (ADR-0100): switches modes on and logs every input chunk it receives.

probe.py LOG [--ruler N] [--modes "1000,1002,1006"] [--alt] [--ctl CTLFILE]
- prints N numbered lines on the main screen first,
- optionally switches to the alternate screen (1049) and draws a few rows there,
- switches the given DEC private modes on,
- logs every input chunk as a JSON line {"t": ..., "hex": ...} to LOG (and {"event": "ready"} once set up),
- polls CTLFILE: each new line is written to the terminal after decoding \\e, \\r and \\n (a driver changes modes this way);
  a line `quit` ends the probe.
"""
import json, os, sys, time, select, termios, tty

args = sys.argv[1:]
log = args[0]
ruler = 0; modes = []; alt = False; ctl = None
i = 1
while i < len(args):
    a = args[i]
    if a == '--ruler': ruler = int(args[i + 1]); i += 2
    elif a == '--modes': modes = [m for m in args[i + 1].split(',') if m]; i += 2
    elif a == '--alt': alt = True; i += 1
    elif a == '--ctl': ctl = args[i + 1]; i += 2
    else: raise SystemExit('bad argument ' + a)

def w(s):
    os.write(1, s.encode())

fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
lf = open(log, 'a', buffering=1)
try:
    for n in range(1, ruler + 1):
        w(f"ruler line {n:04d}\r\n")
    if alt:
        w("\x1b[?1049h\x1b[H\x1b[2J")
        for r in range(1, 6):
            w(f"\x1b[{r};1HALT SCREEN row {r}")
    for m in modes:
        w(f"\x1b[?{m}h")
    w("\x1b[7;1HPROBE READY" if alt else "\r\nPROBE READY\r\n")
    lf.write(json.dumps({"t": time.time(), "event": "ready"}) + "\n")
    pos = 0
    while True:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            data = os.read(fd, 4096)
            if not data:
                break
            lf.write(json.dumps({"t": time.time(), "hex": data.hex()}) + "\n")
        if ctl and os.path.exists(ctl):
            with open(ctl, 'rb') as f:
                f.seek(pos); chunk = f.read(); pos += len(chunk)
            for line in chunk.decode().splitlines():
                if line == 'quit':
                    raise SystemExit
                w(line.replace('\\e', '\x1b').replace('\\r', '\r').replace('\\n', '\n'))
                lf.write(json.dumps({"t": time.time(), "event": "ctl", "line": line}) + "\n")
finally:
    for m in modes:
        w(f"\x1b[?{m}l")
    if alt:
        w("\x1b[?1049l")
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
    lf.write(json.dumps({"t": time.time(), "event": "exit"}) + "\n")
