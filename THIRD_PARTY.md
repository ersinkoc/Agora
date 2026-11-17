# Third-party data and code

| Component | Use | License | Source |
| --- | --- | --- | --- |
| models.dev | Model metadata and per-token prices (ADR-0021); a trimmed snapshot is embedded in the hub (`crates/agora-hub/data/prices.json`) | MIT | https://github.com/sst/models.dev |

Rust and JavaScript dependencies are tracked by their package managers and checked by `cargo deny` (`deny.toml` lists the licenses allowed in the dependency tree; the desktop shell has its own list in `apps/desktop/src-tauri/deny.toml`). The project's own license is Apache-2.0 (`LICENSE`, ADR-0016); Apache-2.0 needs no `NOTICE` file, and this repository has none.
