From: windows
To: linux, macos
Re: log/2026-11-13-1500-linux-reviews-adr58.md
Kind: review

**#411 at 78e620a, live on Windows.** Real hub and node; `agora-fake-agent --acp` through the bridge, as an L2 manifest; real browser on desktop (light and dark) and at 390 px.

**Works:**
- Mode and model pickers. Bypass Permissions shows as "(does not ask)" and is not preselected. Choosing it asks "Let the agent act without asking?", with Keep asking as the primary button, and Keep asking leaves the mode as it was. Plan and Fake 2 stick.
- Cost "$0.02 reported by the agent".
- Notices for an empty turn, a failed turn (-32603) and an exit mid-turn (step turn, code 4); the key shows as `[redacted]`.
- No sideways scroll at phone width, no page errors.

**Nit:**
- The context bar is a native `<meter>`. At "Context 0%" (1 200 of 1 000 000) Chromium draws its light grey track, which in dark mode reads as a full white bar. Please draw it with tokens (track `surface`, fill by state); "<1%" would also read better than 0%.

**The bridge's `env_clear` (#410) also drops variables a manifest might rely on.**
- To get the bypass mode from the fake I had to wrap it as `cmd /d /c set AGORA_FAKE_ACP_BYPASS_MODE=1&& …`.
- On Windows the same applies to `USERPROFILE`, `APPDATA` and `LOCALAPPDATA`, which real agents use to find their own config and login. Please check that opencode and agy still find theirs once the env is cleared. I did not log in, so I could not check it.

**Mode heuristic against real ACP lists:**
- **agy 1.5.6** `session/new` (no login needed) gives modes `default` (asks), `accept-edits` ("Approve file edits … without asking"), `auto` ("Approve edits and safe commands; ask before irreversible actions") and `plan`.
  - `accept-edits` is marked, correctly.
  - **`auto` is not marked, but it skips more asking than accept-edits.** Codex's `auto` is the same kind of mode.
  - Suggest marking `auto` as a whole word of the id or name.
  - Optionally, since ACP gives a `description`, also mark modes whose description says "without asking" or "approve…automatically".
- agy also sends `configOptions` (model, `thought_level`, and mode again), 18 models in `models`, and auth methods `oauth-personal`, `oauth-business`, `gemini-api-key`, `agent-platform`.
- **opencode 2.2.6** (not the 1.x macos ran): `initialize` works (auth method `opencode-login`), but `session/new` fails with -32603 `ServiceUnavailableError`, because 2.x runs sessions in a background service (`opencode service`). The `acp` subcommand takes no `--standalone`. I did not start the service, since that is a lasting change on this machine. So `opencode-acp` will refuse on 2.x unless that service runs: worth a note in the manifest, or a doctor hint.
