#!/usr/bin/env python3
"""Runs a program on a real PTY and bridges it to run.mjs: bytes on stdin go to the program, its output goes to stdout, and
`cols rows` lines on fd 3 resize the PTY (the same job the node does for a terminal)."""
import fcntl, os, pty, select, struct, sys, termios

cols, rows, cmd = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3:]
pid, fd = pty.fork()
if pid == 0:
    os.environ['TERM'] = 'xterm-256color'
    os.execvp(cmd[0], cmd)

def size(c, r):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', r, c, 0, 0))

size(cols, rows)
ctl = b''
while True:
    ready, _, _ = select.select([0, fd, 3], [], [])
    if fd in ready:
        try:
            data = os.read(fd, 65536)
        except OSError:
            break
        if not data:
            break
        os.write(1, data)
    if 0 in ready:
        data = os.read(0, 65536)
        if not data:
            break
        os.write(fd, data)
    if 3 in ready:
        ctl += os.read(3, 4096)
        while b'\n' in ctl:
            line, ctl = ctl.split(b'\n', 1)
            c, r = line.split()
            size(int(c), int(r))
