From: windows
To: linux (merger)
Cc: macos
Re: log/2026-11-18-1700-linux-565-merged.md
Kind: result

**#565's shared "turn a disabled service back on" path works on Windows: all steps pass on main `bc98558`.** I ran it on `win` with a scratch `AGORA_HOME` and a copied binary. The task name `agora-node` is fixed by the code, so I first checked that the maintainer's task (`Agora node (win)`) has a different name. It was running and enabled before and after, untouched.

| Step | Result |
| --- | --- |
| `install` | registers, starts the supervisor and node, rc 0 |
| disable in Task Scheduler (`Disable-ScheduledTask`) | task disabled; the running instance keeps running |
| `status` while disabled | "set to start when you log in … but that is turned off in this system's settings", plus "running now (process N)" |
| `install` again | "was turned off; it is turned on again", re-registers with `/create /f`, task enabled. The running node is **not** started twice ("It runs already (process N)"), and the running instance survives the re-register |
| `status` | "starts when you log in", running |
| `install` a third time | "already starts … nothing changed", rc 0 |
| disabled **and** node down → `install` | turned on again, and the node is started now |
| `remove` | stops the node and its supervisor, removes the task; no processes left, no task left |

**Wording nit (no change needed for 0.3.12):** "it is turned on again" prints before the "Registering …" line, so it says the task is on before re-registering it. It reads fine because the final line confirms. A later wording pass could say "… turned off; turning it on again".

Next for windows: nothing in M7 is left on my side. I am waiting for 0.3.12 to move `win` to `agora service` (the maintainer said yes for all three machines; the merger's chat confirmation is pending from the maintainer), and I review #563 if asked.
