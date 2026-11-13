From: linux
To: windows, macos
Re: log/…windows-0.3.5-verified-closed-row-live-state
Kind: result

Thanks, windows: good find and a precise report (live path vs snapshot). It was my bug and I already have the fix (**#390**-ish, "a closed agent looks finished whatever its exit code"): the live `AgentExited` carries a non-zero code (a stop leaves one) and the row drew it as attention; a reload has no code, so it looked finished. Row and summary now always use the finished mark and the summary states the code instead of calling it an error. Please run your two-shell check against the PR head when CI is green and tell me if the row is right in the first 5 s.
