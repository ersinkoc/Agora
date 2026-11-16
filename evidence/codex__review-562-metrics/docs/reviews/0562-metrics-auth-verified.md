# #562: independent Codex re-check

Reviewed head: `a87f339e042d12ed7ab421c3f897c1951a6af194`. **Approve the corrected metrics authorization and disabled-route behavior.**
The two earlier findings from `comms:d16a836` are addressed; no blocking finding remains in this review scope.

## Authorization before metrics work

[Proof](0562-auth-work-proof.rs) deliberately holds the actual hub mutex while another thread calls the actual HubSessions
metrics method. Requests with no token, an invalid/revoked/expired token return 401; viewer and member return 403. All finish
while that mutex is still held, with empty bodies. An owner's request is a positive control: it waits on that same mutex and
returns 200 with metrics only after release. This tests the ordering by behavior, without a copied authorization implementation.

Temporarily restoring the original metrics method from `f8cc09c` makes this proof fail with
`rejected scrape reached blocked metrics work`. The fixed method passes. `resync_count` uses SQL COUNT(*) rather than loading
rows; the hub mutex is released before the store count and text formatting. Output names and the histogram le labels are fixed,
with aggregate values only, not node/project/person names or paths.

## Disabled endpoint and HEAD

[Real HTTP proof](0562-http-proof.rs) uses the actual embedded-app asset resolver and loopback TCP requests. When disabled,
GET and HEAD to `/api/metrics` and `/api/metrics?x=1`, both anonymous and owner-authenticated, are byte-identical to an unknown
client-side route (200 app shell). When enabled, HEAD has the same headers/status/content length as GET and no body, for the
anonymous 401 and owner 200 cases. The PR's original real-socket test also checks viewer 403 and machine-ingest counters.

Removing only the metrics-on routing condition makes this proof fail on `GET /api/metrics`; the fixed route passes.
`observability.md` records the disabled-route behavior and explicitly states that scraping renews the owner's session.

## Checks and reproduction

- `cargo test -p agora-hub`: **257 passed, 3 ignored**.
- Both additional proofs pass on the reviewed head; both restored defects fail for the expected assertion, not a compiler error.
- `cargo fmt --all -- --check`, `cargo clippy -p agora-hub --all-targets -- -D warnings`, docs and whitespace checks pass.
- Required Linux/macOS/Windows, bench, web, docs and schema CI are all green on the reviewed head.

In a disposable worktree at the reviewed head, run:

```sh
python3 docs/reviews/0562-run-review.py
```

The [runner](0562-run-review.py) appends the proof modules only for these tests, runs the positive and negative cases, and restores
all four product files byte for byte in a finally block. [Structured results](0562-validation.json) include their exit statuses.
The original defective commit is in this branch's history. No production source change is retained or proposed by this review.

This re-check ran on Linux using in-memory stores, synthetic session tokens and loopback sockets. No deployed hub, real accounts
or agent tools were used. The prior release performance comparison was not repeated; this review checks the corrected route/auth
work and reads the metrics hot path. It adds no claim about workloads beyond the PR's existing performance evidence.

The PR description's old "off answers 404" sentence should be refreshed to say it follows the unknown-asset route, as the code and
observability document now do. That description cleanup does not change the approval of this exact source head.
