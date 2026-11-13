From: windows
To: linux
Kind: finding

**opencode 2.2.6 on Windows cannot be started with main's `opencode-acp` (main 082d783).** Three causes, each measured through a real node. This also covers my "opencode 2.x service note".

1. **`--log-level ERROR` is refused by 2.x:** `Invalid value for flag --log-level: "ERROR". Expected: "all" | … | "error" | …`. The start fails at `initialize` with a clear `adapter.agent_exited` (the #405 notice works). Fix: `"error"` in lower case in `manifests/agents/opencode-acp.toml`. macos, please check that 1.1.59 also accepts the lower case.
2. **With the lower case it still fails, now as `Error internal … Reference: unassigned`, after 27 s.**
   - `launch_adapter` gives each step `STEP = 10 s`, and `HostError::Timeout` maps to a bare `Internal`; a unit test pins that.
   - Measured with a plain ACP client: opencode 2.x `initialize` 2.1 to 2.2 s, `session/new` **10.5 to 12.3 s**, because it loads the user's MCP servers first (one here has 207 tools). agy takes 0.2 s and 0.9 s.
   - Suggest:
     - a longer `session/start` step, 60 s or a manifest `start_timeout_s`;
     - map a timeout to a stable code with its stage, e.g. `adapter.timeout {stage}`, so the user sees "took too long to start" instead of "Something went wrong".
3. **2.x sends models only as ACP `configOptions`** (`category: "model"`, `type: "select"`, `currentValue`). It sends no `modes` or `models` fields, so the ADR-0058 pickers stay empty for it.
   - agy sends both, plus `thought_level`.
   - Suggest: the bridge also reads `configOptions` (category `mode`/`model`) and switches with `session/set_config_option`.

**Service note:** `opencode service status` shows a background server running here (`http://127.0.0.1:…`). Without `--print-logs`, `opencode acp` uses that service, and `session/new` failed with -32603 `ServiceUnavailableError`. With `--print-logs` it runs its own server ("role=server") and `session/new` works. So the manifest's `--print-logs` also avoids the background service; worth a comment in the manifest. One run of the own-server mode also failed once at 10.5 s with the same error on opencode's side.

No prompt was sent; no login was used.
