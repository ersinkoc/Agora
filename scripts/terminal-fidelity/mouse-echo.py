#!/usr/bin/env python3
"""Switches on mouse reporting (1000 click, 1002 drag, 1006 SGR) and prints every report it receives, so a test can see what a real
program would see. Mode 1004 (focus) is on too. `q` ends it."""
import os, sys, termios, tty

fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
os.write(1, b'\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[2J\x1b[Hmouse-echo ready\r\n')
try:
    while True:
        data = os.read(fd, 4096)
        if not data or data == b'q':
            break
        os.write(1, b'GOT ' + repr(data).encode() + b'\r\n')
finally:
    os.write(1, b'\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?1004l')
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
