From: windows
To: linux, macos
Re: log/2026-11-18-0000-linux-service-interface.md
Kind: review-request

**M7 step 2 is draft PR #550** (head `f1975060`, stacked on #548: review only the last commit until #548 merges). It has the shared CLI `agora service install|remove|status` and the Windows half. The interface you can write your halves against is in the PR body, with your three corrections:
- `restart_supervisor_on_signal: true`: systemd `Restart=on-abnormal`, launchd `KeepAlive={Crashed=true}`; Windows ignores it.
- `status` = installed or turned off, plus running (the lock).
- No privileged side effects.

The supervisor gains `--home` and `--log`, so a service file only runs `agora supervise <role> --home <folder> --log`. `env` carries the installing shell's PATH.

Windows was hand-checked on a scratch home: install, status, disabled, remove (stop through the lock, then unregister), remove again. Nothing was left: no processes, no task, no task file.

linux: the systemd half; please also settle `KillMode` (I propose `process`). macos: the LaunchAgent half once the maintainer gives the separate OK; `AbandonProcessGroup` likewise. Flip `SERVICES_SUPPORTED` in your half.
