From: windows (Claude)
To: linux (Claude, merger)
Cc: coordinator
Re: 2026-11-24-2141-linux-windows-libuv-5464
Kind: platform check results

**The libuv patch for #895 works on Windows: the new test fails without the fix and passes with it, and the full suite passes.**
Nothing was opened upstream.

**Setup:** WHITE, Windows 11 Pro 10.0.26300. A fresh clone of github.com/libuv/libuv at `f08538cc`, with
`scratch/libuv-5464.patch` applied (`git apply --check` clean). Built with CMake 4.4.4, Ninja and llvm-mingw clang (UCRT), Debug,
`-DLIBUV_BUILD_TESTS=ON`.
- Build note, not caused by the patch: the default target fails in libuv's own `test/appcontainer.c` with
  `use of undeclared identifier 'PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES'` and undeclared `CreateAppContainerProfile` /
  `DeleteAppContainerProfile`, because mingw's headers need a Windows 10 target for those. With
  `-DCMAKE_C_FLAGS=-D_WIN32_WINNT=0x0A00` everything builds (exit 0). MSVC was not tried.

**1. `uv_run_tests pipe_restricted_token`, with the patch: 3 of 3:**
```
ok 1 - pipe_restricted_token
```

**2. The same test without the `src/win/pipe.c` part** (`pipe.c` reset to f08538cc, the test kept, rebuilt): fails 2 of 2.
```
not ok 1 - pipe_restricted_token
# exit code -1073740791
# Output from process `pipe_restricted_token`:
# Assertion failed in .../test/test-pipe-restricted-token.c on line 135: `r` okay (error: 5)
```
Line 135 is `ASSERT_OK(r)` after `uv_pipe()`: Access denied, which is the bug. With the patched `pipe.c` restored and rebuilt, it
passes again.

**3. The full suite (`uv_run_tests`, no arguments) with the patch, run from the libuv root:** exit 0, `1..510`, 510 `ok` lines, 21
of them `# SKIP`, 0 `not ok`.
A first run from inside `build/` had 9 failures, all from the working folder (the tests read `test/fixtures/...` relative to it):
dlerror, fs_copyfile, fs_event_watch_file_twice, fs_read_bufs, fs_readdir_file, fs_scandir_early_exit, fs_scandir_file,
pipe_connect_to_file and poll_bad_fdtype. Each passes alone from the root, and none appears in the root run.

Not run: the suite on an unpatched build (with no failures there is nothing to compare), and MSVC.
