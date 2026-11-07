//! Process inspection through `libproc`.

use std::ffi::CStr;
use std::io;
use std::path::PathBuf;

pub(crate) fn name(pid: u32) -> io::Result<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for `buf.len()` bytes.
    let n = unsafe { libc::proc_name(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

pub(crate) fn cwd(pid: u32) -> io::Result<PathBuf> {
    // SAFETY: an all-zero proc_vnodepathinfo is a valid (empty) value for the kernel to fill.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    // SAFETY: the buffer is a live proc_vnodepathinfo of `size` bytes.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast(),
            size,
        )
    };
    if n <= 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel NUL-terminates the path inside the fixed-size array.
    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    Ok(PathBuf::from(path.to_string_lossy().into_owned()))
}

pub(crate) fn children(pid: u32) -> io::Result<Vec<u32>> {
    let mut buf = vec![0i32; 1024];
    loop {
        buf.iter_mut().for_each(|p| *p = 0);
        // SAFETY: the buffer is valid for the byte size passed.
        let r = unsafe {
            libc::proc_listchildpids(
                pid as i32,
                buf.as_mut_ptr().cast(),
                (buf.len() * std::mem::size_of::<i32>()) as i32,
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        // Whether the return value counts bytes or pids differs between libproc versions, so read the
        // zero-initialised buffer instead: unused slots stay 0. A full buffer may be truncated, so grow it.
        if buf.last().is_some_and(|p| *p != 0) {
            buf = vec![0i32; buf.len() * 2];
            continue;
        }
        let mut out: Vec<u32> = buf.iter().filter(|p| **p > 0).map(|p| *p as u32).collect();
        out.sort_unstable();
        return Ok(out);
    }
}
