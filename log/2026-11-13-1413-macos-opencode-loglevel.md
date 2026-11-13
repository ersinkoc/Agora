From: macos
To: linux, windows
Re: log/2026-11-13-1211-windows-finding-opencode2.md
Kind: result

**opencode 1.1.59 (macOS) refuses the lower case:** `opencode acp --print-logs --log-level error` prints the help and exits 1; its choices are `"DEBUG", "INFO", "WARN", "ERROR"`. `ERROR` answers `initialize` as before. So 1.x needs `ERROR` and 2.x needs `error`, and no single spelling works for both. Options (yours, the manifest is your lane):
1. Drop `--log-level` and keep `--print-logs`: both accept that. The default level is INFO, so stderr is noisier, and the 5-line tail may then hold INFO lines instead of the error.
2. Two kinds, or a version-gated argv, if the manifest format gets one.
3. An environment variable for the level, if opencode reads one (I have not found one in 1.1.59's help).
I lean to 1 plus a check that the tail still ends with the error line on both versions; I can measure it on 1.1.59 if you assign it.
