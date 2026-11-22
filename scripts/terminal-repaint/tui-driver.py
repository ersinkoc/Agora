#!/usr/bin/env python3
"""Drives `agora tui` in a pty for the repaint check (ADR-0100): opens the first agent's terminal (Enter) after the probe already
switched its modes on (late attach), sends what the user's terminal would send for a click and a wheel notch, and prints one JSON
line per check with the bytes the probe received. Standard library only. Every wait is for a condition, with a deadline.

tui-driver.py BIN HUBDIR LOG CTL MODE   (AGORA_HOME etc. from the environment; MODE: mouse | alt)
"""
import codecs, fcntl, json, os, pty, re, select, signal, struct, sys, termios, time, unicodedata

BIN, HUBDIR, LOG, CTL, MODE = sys.argv[1:6]
COLS, ROWS = 120, 40
pid, fd = pty.fork()
if pid == 0:
    os.execvp(BIN, [BIN, 'tui', '--hub-dir', HUBDIR])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', ROWS, COLS, 0, 0))

# One escape sequence, control character or run of text. An escape cut off at the end of a read matches nothing and waits for
# the rest.
TOKEN = re.compile(r'\x1b\[([0-9;?<>=]*)[ -/]*([@-~])|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()*+].|\x1b[^\[\]()*+]'
                   r'|([\x00-\x1a\x1c-\x1f\x7f])|([^\x00-\x1f\x7f]+)')


class Screen:
    """What the console shows now: its output applied to a grid of the window's size. The waits look at this, never at everything
    the console ever wrote: a frame that was drawn and then drawn over (a pane that showed the program for one frame) no longer
    counts. Only what the console uses to draw is followed (cursor moves, erases, text); styles and modes are skipped."""

    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.grid = [[' '] * cols for _ in range(rows)]
        self.r = self.c = 0
        self.decode = codecs.getincrementaldecoder('utf-8')('replace').decode
        self.rest = ''

    def newline(self):
        if self.r == self.rows - 1:
            self.grid = self.grid[1:] + [[' '] * self.cols]
        else:
            self.r += 1

    def erase(self, row, start, end):
        self.grid[row][start:end] = [' '] * (end - start)

    def csi(self, params, final):
        if params[:1] in ('?', '<', '>', '='):
            return
        n = [int(p) if p else 0 for p in params.split(';')]
        first = n[0] or 1
        if final in 'Hf':
            self.r = min(max(first, 1), self.rows) - 1
            self.c = min(max(n[1] if len(n) > 1 and n[1] else 1, 1), self.cols) - 1
        elif final in 'ABCD':
            dr, dc = {'A': (-first, 0), 'B': (first, 0), 'C': (0, first), 'D': (0, -first)}[final]
            self.r = min(max(self.r + dr, 0), self.rows - 1)
            self.c = min(max(self.c + dc, 0), self.cols - 1)
        elif final == 'G':
            self.c = min(first, self.cols) - 1
        elif final == 'd':
            self.r = min(first, self.rows) - 1
        elif final == 'J':
            rows = {0: range(self.r + 1, self.rows), 1: range(0, self.r), 2: range(self.rows)}.get(n[0], ())
            for row in rows:
                self.erase(row, 0, self.cols)
            if n[0] == 0:
                self.erase(self.r, self.c, self.cols)
            elif n[0] == 1:
                self.erase(self.r, 0, self.c + 1)
        elif final == 'K':
            start, end = {0: (self.c, self.cols), 1: (0, self.c + 1)}.get(n[0], (0, self.cols))
            self.erase(self.r, start, min(end, self.cols))
        elif final == 'X':
            self.erase(self.r, self.c, min(self.c + first, self.cols))

    def text(self, run):
        for ch in run:
            if unicodedata.combining(ch):
                continue
            w = 2 if unicodedata.east_asian_width(ch) in 'WF' else 1
            # The cursor stays on the last column after it is written (a terminal wraps at the next character, not before).
            if self.c + w > self.cols:
                self.c = 0
                self.newline()
            self.grid[self.r][self.c] = ch
            if w == 2:
                self.grid[self.r][self.c + 1] = ''
            self.c += w

    def feed(self, data):
        s = self.rest + self.decode(data)
        at = 0
        while at < len(s):
            m = TOKEN.match(s, at)
            if not m:
                break
            at = m.end()
            if m.group(2):
                self.csi(m.group(1), m.group(2))
            elif m.group(3):
                ctl = m.group(3)
                if ctl == '\r':
                    self.c = 0
                elif ctl == '\n':
                    self.newline()
                elif ctl == '\b':
                    self.c = max(self.c - 1, 0)
            elif m.group(4):
                self.text(m.group(4))
        # An unfinished escape waits for the rest of it (one that never ends is dropped rather than stalling the screen).
        self.rest = s[at:] if len(s) - at < 256 else ''

    def lines(self):
        return [''.join(row).rstrip() for row in self.grid]

    def shows(self, pattern):
        return any(re.search(pattern, line) for line in self.lines())


screen = Screen(COLS, ROWS)
received_bytes = 0
quiet_since = time.time()


def read_some(t=0.05):
    global received_bytes, quiet_since
    r, _, _ = select.select([fd], [], [], t)
    if r:
        try:
            data = os.read(fd, 65536)
        except OSError:
            return
        screen.feed(data)
        received_bytes += len(data)
        quiet_since = time.time()


def shown():
    """The console's current screen, its non-empty rows, for a failure message."""
    return ' ⏎ '.join(re.sub(r'\s+', ' ', l).strip() for l in screen.lines() if l.strip())


def until(what, ok, deadline=30.0, settled=False):
    """Waits until `ok()` holds; with `settled`, also until the console has been quiet for half a second (it holds for the frame
    the console stopped at, not only for one it drew on the way)."""
    end = time.time() + deadline
    while not (ok() and (not settled or time.time() - quiet_since >= 0.5)):
        if time.time() > end:
            raise SystemExit(f'timed out waiting for {what}; the console shows: {shown()}')
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
    # Enter opens the selected agent, so it waits until the list on screen has the agent's row (`shell · <name>`): the console
    # draws before the hub's view of the agents arrives, and an Enter on the empty list opens nothing. The console selects the
    # first agent of the list it drew before it reads the key, so the row on screen is the one Enter opens.
    until('the agent to be listed', lambda: screen.shows(r'shell · \S') and not screen.shows(r'No agents yet\.'), settled=True)
    opened = received_bytes
    os.write(fd, b'\r')
    # The pane shows the program once the node's repaint has arrived (and with it the program's modes), and still shows it when
    # the console has stopped drawing.
    until('the pane to show the probe', lambda: received_bytes > opened and screen.shows(r'PROBE READY'), settled=True)
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
