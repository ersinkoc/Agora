# Usage, cost, and quotas

Goal: per agent, project, node, and account: tokens in/out, estimated cost, and subscription quota status.
Three separate data sets, each with its own source of truth.

## 1. Usage (what was consumed)

Sources, in order of preference:
1. Adapter or ACP-reported usage (`usage/report`), including the model id.
2. Telemetry the agent exports (for example OpenTelemetry). The node runs a local OTLP receiver and injects the
   exporter configuration at launch (`[usage] source = "otel"`).
3. Agent-local usage logs parsed per manifest (`[usage] source = "file"`).
4. None: show "usage unavailable" rather than guessing.

Usage records: `{ agent_id, account, model_id, input_tokens, output_tokens, cache_read, cache_write, at }`.

## 2. Prices and models (what it costs) — models.dev

Decided in ADR-0021.

- Catalog: `https://models.dev/api.json`, fetched by the hub, cached, with a bundled snapshot per release.
- Cost estimate = usage tokens × catalog prices per million tokens (input, output, cache read, cache write).
- Model id mapping: the manifest or adapter maps the agent's reported model id to a models.dev
  `provider/model` id (`[usage.model_map]` in the manifest).
- Overrides: hub-local table for negotiated prices and private models.
- If the agent reports its own cost, that value wins and the estimate is shown only as a comparison.
- Gaps found in models.dev are contributed upstream.

## 3. Quotas (how much is left) — quota profiles

Subscription plans (for example rolling multi-hour windows, weekly caps, per-plan message or credit limits) are
not in models.dev and change often. They are kept as **quota profiles**: versioned data files distributed
through the adapter registry, so they update without a release, exactly like agent manifests.

```toml
id = "example-provider/pro"
display_name = "Example Pro"
version = "2026.11.07.1"
last_verified = "2026-11-07"
confidence = "community"        # official | observed | community | unknown

[[windows]]
kind = "rolling"                # rolling | calendar_week | calendar_month | fixed
duration = "5h"
unit = "messages"               # messages | tokens | requests | credits | usd
limit = 0                       # 0 = unknown; show usage without a bar

[[sources]]
url = "https://example.com/plans"
note = "Official plan page"
```

Precedence for what the window shows:
1. Quota status reported by the agent itself (adapter, ACP, or parsed agent output/limits messages).
2. Observed limits: when an agent reports a rate-limit or quota-exhausted event, the node records the
   consumption at that moment; repeated observations calibrate the profile locally.
3. The quota profile.
4. Unknown: show consumption only, never an invented limit.

Every quota number in the UI shows its confidence and last verification date.

### Building the quota knowledge over time

Quota facts are researched continuously, not decided once (backlog `research.md`):
- Survey open-source projects that already track usage and limits for each agent (log parsers, usage
  dashboards, status-line tools) and record what they read and how.
- Read official plan and rate-limit documentation per provider.
- Collect observed limit events from opt-in users (anonymized: plan id, window, consumption at limit).
- Every profile entry cites its sources; disagreements are recorded, not hidden.

## Implementation status
`agora-hub::usage`: models.dev catalog parsing, overrides, exact micro-USD estimates, reported-cost-wins, totals that keep
reported, estimated and unpriced apart. ADR-0033 is built: the hub embeds a trimmed models.dev snapshot (`crates/agora-hub/data/prices.json`, rebuilt by
`scripts/update-prices.sh`), `agora hub prices update` fetches the live catalog on request into `<hub dir>/prices-catalog.json`, and
`<hub dir>/prices.json` overrides (USD per million tokens) sit on top. A bare model id (what agents report) takes its first-party
provider's price, or the only provider that lists it; several resellers with different prices give no estimate. The
`usage.report` command returns tokens and the estimate per agent; the window shows it in Focus and per machine, always as an
estimate with the price date. Not yet: quota profiles, per-project and per-day totals, budget alerts, agent-reported costs.

## Views

Totals per project and day; per agent session; per account; quota bars with confidence; budget alerts through
notification rules.
