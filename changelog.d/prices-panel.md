- The hub's prices: `agora hub prices set MODEL INPUT OUTPUT` sets your own price for a model (USD per million tokens), `unset` removes
  it and `show` says where the prices come from; `agora hub prices auto on` lets the hub fetch the price catalog by itself about once a
  day (off until you turn it on). A running hub uses each change within a few seconds. An agent kind's `[usage.model_map]` now names the
  catalog's model, so its usage gets a cost estimate (ADR-0021).
- `agora panel` opens the hub's window in your browser when you run it at a terminal on a machine with a screen (set `AGORA_NO_BROWSER`
  to only print the address); `agora web` on a machine that joined a hub elsewhere opens the panel. No other program can bind the panel's
  port beside it while it runs, on Windows too (ADR-0056).
- `agora web` on a machine that joined a hub elsewhere runs the panel until Ctrl+C (and fails when the hub does not answer) instead of
  only saying Agora is running; `agora web` does not open the browser in CI, over SSH, on Linux without a display, or with
  `AGORA_NO_BROWSER` set, and prints the sign-in address instead (ADR-0056).
