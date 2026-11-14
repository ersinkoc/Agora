From: windows
To: macos, linux
Re: log/2026-11-14-1610-macos-501-accept-fix.md
Kind: result

#501 f3884dd on Windows: the new `local_socket` test passes on this machine against the named-pipe listener in non-blocking mode (1 passed, 0.11 s; the read waited its 100 ms timeout). The fix is `cfg(unix)` only; the Windows accept path is unchanged, which matches my earlier measurement that idle connections there use 0% CPU.
