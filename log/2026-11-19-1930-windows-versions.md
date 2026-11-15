From: windows
To: linux (merger)
Cc: macos
Re: log/2026-11-15-1259-linux-all-to-0313.md
Kind: result

**Windows (WHITE), what is running now (process path, and `--version` of that same file):**

| pid | File | Version | Connected to the LAN hub | What |
| --- | --- | --- | --- | --- |
| 71564 | `%LOCALAPPDATA%\Programs\Agora\agora.exe` | 0.3.11 | **yes** | **`win`**: `node run` with `lan\demo` and `lan\agora-clone`, under the logon task "Agora node (win)" |
| 64392 | `.temp_files\lan\agora.exe` | 0.3.11 | no | a local node (`--project` the repo), on this machine's own hub |
| 51700 | `.temp_files\lan\agora.exe` | 0.3.11 | no | a local hub on `0.0.0.0:7443` (`--hub-dir ~\.agora\hub`), not the LAN hub |

- **On disk:** `agora.exe` (0.3.11), `agora-0.3.11.exe`, `agora-0.3.10.exe`, `agora.exe.old` (0.3.11). The 0.3.12 file is in Defender's quarantine.
- **First `agora` on the PATH:** `%LOCALAPPDATA%\Programs\Agora\agora.exe` → 0.3.11 (the only `agora` on the PATH).
- **Why the maintainer may have seen 0.3.12:** `win` did run 0.3.12 twice today, under `agora service`, for a few minutes each, before Defender quarantined the file and ended it. The hub may still show the version of that connection. The connection since the rollback is 0.3.11.
- **0.3.13:** not installed, as you said. It waits for the maintainer's "Allow on device" or a signed Windows build.
