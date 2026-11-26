# 0021. models.dev as the model and price catalog
Status: Accepted
Date: 2026-11-07

## Context
Usage and cost views need model metadata (names, providers, context and output limits, modalities) and token
prices. Maintaining our own catalog is ongoing work that an existing open-source project already does.
models.dev is an MIT-licensed, community-maintained database of model specifications, pricing, and
capabilities, published as JSON (`https://models.dev/api.json`) and stored upstream as TOML files.

Subscription quotas (rolling windows, weekly caps, plan tiers) are not model facts and are not in models.dev.

## Decision
- Use models.dev as the single source for model metadata and per-token prices.
- The hub fetches the catalog periodically, validates it, and caches it. A snapshot is bundled in each release
  so offline and air-gapped hubs work.
- A hub-local override table takes precedence (negotiated enterprise prices, private or self-hosted models).
- Agent-reported model ids are mapped to models.dev `provider/model` ids by a mapping table owned by the agent
  manifest or adapter; unmapped models show usage without cost.
- Missing or wrong data is fixed upstream in models.dev by pull request rather than patched locally long-term.
- Subscription quotas are a separate data set: quota profiles (see `design/usage-and-cost.md`).
- Attribution for models.dev data is listed in `THIRD_PARTY.md`.

## Consequences
- No catalog maintenance in this project beyond mappings and overrides.
- Cost is always an estimate based on catalog prices; reported costs from agents, when available, win.
- Catalog schema changes upstream are handled by a tolerant parser and a CI job that validates the latest
  catalog against our expectations.

## Build status
Built (roadmap M7.6): the catalog bundled with each release, `agora hub prices update` (ADR-0033), and the periodic fetch, which ADR-0033
keeps off until the owner asks: `agora hub prices auto on [--url URL]` writes `<hub dir>/prices-auto`, and the running hub then fetches
about once a day (spread by chance, sooner after a failure with a doubling wait up to 6 h) over the bounded HTTPS client
(`agora-hub::deliver::get`: no redirect, a time cap; 64 MiB here), keeps only a catalog with prices and, when automatic, with at least half
the prices of the one in use, and writes it whole (`crates/agora/src/pricefeed.rs`). The hub-local override table is `<hub dir>/prices.json`,
edited with `agora hub prices set|unset` and shown with `agora hub prices show`; it wins over the catalog, also for the bare id an agent
reports. A running hub takes a change of either file within 5 s. The manifest's `[usage.model_map]` is read (at most 256 entries of at
most 128 bytes, the value `provider/model`) and the node reports the mapped id with the agent's usage, from a usage file and from an
adapter alike; the ids of session options are never mapped. Not built: the CI job that validates the live catalog, quota profiles.
