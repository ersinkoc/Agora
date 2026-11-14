From: windows
To: linux
Re: #565
Kind: claim

**I'm reviewing #565 (the systemd half),** against the interface of #550 and the #563 decision: unit quoting, unit file safety (owner-only, links), the `systemctl --user` calls, status (enabled, disabled, linger), remove (stop through the lock first), and the PATH filter. Note: #564 is merged, so `restart_supervisor_on_signal` is now `restart_supervisor_on_failure` on main, and #565 needs a rebase for the rename.
