//! `restore_terminal_on_stop`: a stop signal writes the reset bytes, puts the terminal's settings back and then ends the process by
//! that signal, whatever the program was doing (here: sleeping, as a loop that never looks at a flag would).
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Read};
use std::os::fd::FromRawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

const RESET: &[u8] = b"\x1b[?1000l\x1b[?1049l";

/// The child's side: raw mode, the handler, then sleep. Does nothing unless the parent test started it.
#[test]
fn stop_child() {
    if std::env::var_os("AGORA_STOP_CHILD").is_none() {
        return;
    }
    let raw = agora_platform::RawMode::enable().unwrap();
    agora_platform::restore_terminal_on_stop(&raw, RESET).unwrap();
    println!("READY");
    std::thread::sleep(std::time::Duration::from_secs(30));
}

fn echo_on(fd: i32) -> bool {
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(fd, &mut t) }, 0);
    t.c_lflag & libc::ECHO != 0 && t.c_lflag & libc::ICANON != 0
}

#[test]
fn a_stop_signal_restores_the_terminal_and_still_ends_the_process_by_that_signal() {
    for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
        let (mut master, mut slave) = (0, 0);
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        assert!(echo_on(slave));
        let child_in = unsafe { Stdio::from_raw_fd(libc::dup(slave)) };
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "stop_child", "--nocapture", "--test-threads=1"])
            .env("AGORA_STOP_CHILD", "1")
            .stdin(child_in)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        while !line.contains("READY") {
            line.clear();
            assert!(out.read_line(&mut line).unwrap() > 0, "child ended early");
        }
        assert!(!echo_on(slave), "the child put the terminal in raw mode");
        unsafe { libc::kill(i32::try_from(child.id()).unwrap(), sig) };
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(sig), "ended by the same signal");
        let mut rest = Vec::new();
        out.read_to_end(&mut rest).unwrap();
        assert!(
            rest.windows(RESET.len()).any(|w| w == RESET),
            "the reset bytes were written: {rest:?}"
        );
        assert!(echo_on(slave), "the terminal's settings are back");
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
    }
}
