#!/usr/bin/env python3
"""Drives `agora tui` in a pty for the repaint check (ADR-0100): opens the first agent's terminal (Enter) after the probe already
switched its modes on (late attach), sends what the user's terminal would send for a click and a wheel notch, and prints one JSON
line per check with the bytes the probe received. Standard library only.

tui-driver.py BIN HUBDIR LOG CTL MODE   (AGORA_HOME etc. from the environment; MODE: mouse | alt)
"""
import fcntl, json, os, pty, select, signal, struct, sys, termios, time

BIN, HUBDIR, LOG, CTL, MODE = sys.argv[1:6]
COLS, ROWS = 120, 40
pid, fd = pty.fork()
if pid == 0:
    os.execvp(BIN, [BIN, 'tui', '--hub-dir', HUBDIR])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', ROWS, COLS, 0, 0))

def pump(t):
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                os.read(fd, 65536)
            except OSError:
                return

def lines():
    return [json.loads(l) for l in open(LOG).read().strip().split('\n')]

def act(seq, wait=1.0):
    m = len(lines())
    os.write(fd, seq)
    pump(wait)
    return ''.join(l['hex'] for l in lines()[m:] if 'hex' in l)

def report(name, got):
    print(json.dumps({"name": name, "hex": got}), flush=True)

try:
    pump(5)
    os.write(fd, b'\r')
    pump(6)
    # A click and a wheel notch in the middle of the window, inside the agent's pane, as SGR reports from the user's terminal.
    if MODE == 'mouse':
        report('first click', act(b'\x1b[<0;60;20M\x1b[<0;60;20m'))
        report('wheel', act(b'\x1b[<64;60;20M'))
        report('click', act(b'\x1b[<0;60;20M\x1b[<0;60;20m'))
    else:
        report('wheel', act(b'\x1b[<64;60;20M'))
    os.write(fd, b'\x1d'); pump(1)
    os.write(fd, b'q'); pump(1)
finally:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
