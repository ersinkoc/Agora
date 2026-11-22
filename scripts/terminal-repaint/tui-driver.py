#!/usr/bin/env python3
"""Drives `agora tui` in a pty for the repaint check (ADR-0100): opens the first agent's terminal (Enter) after the probe already
switched its modes on (late attach), sends what the user's terminal would send for a click and a wheel notch, and prints one JSON
line per check with the bytes the probe received. Standard library only. Every wait is for a condition, with a deadline.

tui-driver.py BIN HUBDIR LOG CTL MODE   (AGORA_HOME etc. from the environment; MODE: mouse | alt)
"""
import fcntl, json, os, pty, re, select, signal, struct, sys, termios, time

BIN, HUBDIR, LOG, CTL, MODE = sys.argv[1:6]
COLS, ROWS = 120, 40
pid, fd = pty.fork()
if pid == 0:
    os.execvp(BIN, [BIN, 'tui', '--hub-dir', HUBDIR])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', ROWS, COLS, 0, 0))
screen = bytearray()

def read_some(t=0.05):
    r, _, _ = select.select([fd], [], [], t)
    if r:
        try:
            screen.extend(os.read(fd, 65536))
        except OSError:
            pass

def visible_text():
    """The console's output with escape sequences removed, for a failure message."""
    text = re.sub(rb'\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07]*\x07|\x1b.', b' ', bytes(screen))
    return re.sub(r'\s+', ' ', text.decode('utf-8', 'replace'))[-1500:]

def until(what, ok, deadline=30.0):
    end = time.time() + deadline
    while not ok():
        if time.time() > end:
            raise SystemExit(f'timed out waiting for {what}; the console showed: {visible_text()}')
        read_some()

def lines():
    return [json.loads(l) for l in open(LOG).read().strip().split('\n') if l]

def received(mark):
    return ''.join(l['hex'] for l in lines()[mark:] if 'hex' in l)

def act(seq, want):
    """Sends `seq` and waits until the probe received `want` hex characters (each report is complete by then)."""
    m = len(lines())
    os.write(fd, seq)
    try:
        until('the probe to receive the report', lambda: len(received(m)) >= want, 10.0)
    except SystemExit:
        pass
    return received(m)

def report(name, got):
    print(json.dumps({"name": name, "hex": got}), flush=True)

try:
    def settle(since):
        """Waits for output after offset `since`, then until the console has been quiet for half a second (it has drawn)."""
        until('the console to draw', lambda: len(screen) > since)
        quiet, size = time.time(), len(screen)
        while time.time() - quiet < 0.5:
            read_some()
            if len(screen) != size:
                size, quiet = len(screen), time.time()

    # Enter opens the selected agent, so it waits until the agent is listed: the console draws before the hub's view of the agents
    # arrives, and an Enter before that selects nothing (the pane then never opens). The list names the agent by its kind.
    until('the agent to be listed', lambda: b'shell' in screen)
    settle(0)
    opened = len(screen)
    os.write(fd, b'\r')
    # The pane shows the probe once the node's repaint has arrived (and with it the program's modes).
    until("the pane to show the probe", lambda: b'PROBE' in screen)
    settle(opened)
    if MODE == 'mouse':
        # A click and a wheel notch in the middle of the window, inside the agent's pane, as SGR reports from the user's terminal.
        report('click', act(b'\x1b[<0;60;20M\x1b[<0;60;20m', len('\x1b[<0;1;1M\x1b[<0;1;1m') * 2))
        report('wheel', act(b'\x1b[<64;60;20M', len('\x1b[<64;1;1M') * 2))
    else:
        report('wheel', act(b'\x1b[<64;60;20M', len('\x1b[A' * 3) * 2))
    os.write(fd, b'\x1d')
    os.write(fd, b'q')
finally:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
