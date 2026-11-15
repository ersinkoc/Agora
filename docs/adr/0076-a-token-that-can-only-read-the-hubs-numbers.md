# 0076. A token that can only read the hub's numbers
Status: Proposed
Date: 2026-11-15

## Context
`GET /api/metrics` (architecture review R8, #562) gives the hub's own numbers in the Prometheus text format. It is off unless the
hub runs with `--metrics`, and then only the owner's bearer token reads it. A scraper (Prometheus, a cron `curl`, a home dashboard)
must therefore hold the **owner's session token**. That token can do everything the owner can: approve, start agents, forget
machines, read every project, and sign in a browser. It sits in a scraper's config file, often on another machine, often in a
backup. And every scrape renews the owner's session, as any use does (`docs/design/observability.md`): a scraper keeps that
full-power token alive for as long as it scrapes.

The numbers themselves are aggregates. They hold no names, paths, prompts or terminal bytes: events stored, lock and ingest times,
machines known and online, windows following, approvals waiting, resyncs, relay drops, terminal bytes in total. A token that
reads only these numbers leaks much less than a token that can act.

## Decision
1. **A metrics token: a separate kind of credential that does one thing.** It is accepted only by `GET` and `HEAD` on
   `/api/metrics`. Every other path and every WebSocket treats it as an unknown token (401), exactly as a made-up one. It never
   starts or renews a session, and it carries no user and no role.
2. **Made on the hub's machine, by its owner, from the command line** (like `agora hub code`):
   - `agora hub metrics-token create NAME [--expires DAYS]` prints the token once. It starts with `agm_` so it is recognisable in
     a config file and in a secret scanner. It holds 256 random bits.
   - `agora hub metrics-token list` shows each token's name, when it was made, when it was last used (rounded to the minute) and
     when it expires. It never shows a token.
   - `agora hub metrics-token revoke NAME` makes the token fail at its next use. Revoking is immediate: the hub looks the token up
     on every scrape.
3. **Stored as a hash only.** A new hub table holds `metrics_tokens (name, token_hash, created_ms, expires_ms, last_used_ms,
   revoked_ms)`, with SHA-256 as for sessions. The hub database migrates forward as for every table. A leaked database or backup
   holds no usable token. It is looked up apart from sessions, so it can never match a session, and a session can never match it.
4. **Same gates as today.** The endpoint is still off without `--metrics`: answered byte for byte as an unknown path, for any
   token. The owner's session token keeps working. The token is checked before anything is counted (the #562 review rule).
   A wrong bearer on `/api/metrics` now counts toward the hub's sign-in lockout for that address (today this path does not count
   failures). Guessing a 256-bit token is hopeless anyway, but a burst of failures says someone is trying.
5. **Audit.** Creating and revoking a token are audit events, with the name and never the token. A scrape is not audited (one a
   minute would flood the trail); `last_used_ms` is written at most once a minute.
6. **No expiry by default.** Scrapers run unattended for months, and an expiry nobody notices turns into a silent gap in the
   graphs. `--expires DAYS` is there for those who want one, and `list` shows it.
7. **Not built:** a web page to manage tokens, and per-token rate limits. The lockout of decision 4 covers guessing, and a scrape
   costs one read of counters already kept.

## Alternatives considered
- **Keep the owner's token.** Rejected: that is the problem above.
- **A viewer user for the scraper.** Rejected: a viewer sees every project, agent and machine name over the WebSocket, its
  session expires (the scraper then fails silently) and renews on use, and it is a person-shaped account for a program.
- **A client certificate (mTLS) for scrapers.** Rejected for now: the hub already issues certificates for machines, but asking a
  person to configure Prometheus with a client certificate is far more work than a bearer line, and it would need its own
  revocation path. It stays possible later.
- **No token on loopback.** Rejected: any local user and any local process (an agent included) could read it, and a reverse
  proxy makes every request look local.

## Consequences
- A scraper's config holds a credential that can only read aggregates. Losing it reveals how busy the hub is, not what anyone
  does, and `revoke` ends it.
- One new table and three CLI subcommands, plus their locale lines (en, tr). No protocol message and no schema change: this is the
  hub's HTTP surface and its own database.
- Threat model: a new row. A metrics token that leaks lets the holder read the hub's aggregate numbers until revoked (or until its
  expiry). Mitigations: it is limited to one path, stored hashed, revocable, behind `--metrics`, and failures count toward the lockout.
- `docs/design/observability.md` replaces "A read-only scrape token is a later slice" with how to make one.
- Tests:
  - a metrics token reads `/api/metrics`, but gets 401 on every other HTTP path and on the WebSocket;
  - a revoked or expired token is refused at its next use;
  - with `--metrics` off, a token is answered as an unknown path;
  - the token is never in the database, the audit trail or `list`.

## Open questions for the maintainer
1. CLI only in the first slice (proposed), or also a Settings page in the window?
2. No expiry by default (proposed), or 90 days?
