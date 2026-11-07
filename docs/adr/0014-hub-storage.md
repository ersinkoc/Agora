# 0014. Hub storage
Status: Proposed
Date: 2026-11-06

## Decision (proposed)
- A storage trait in `agora-hub` with an append-only event log, projections, and key-value settings.
- First backend: embedded SQLite (WAL mode). Covers local and single-team server deployments.
- Postgres backend added for team/cloud mode (Phase 7) behind the same trait.
- Event schema avoids SQLite-only features so data can be migrated.

## Consequences
The local hub stays a single file to back up; cloud mode does not require a rewrite.
