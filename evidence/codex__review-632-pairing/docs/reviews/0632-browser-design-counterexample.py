"""Executable trace of section 6 as written, not a server implementation test."""
import json

pending = {}
notifications = []
tokens = {}
address = "192.0.2.10"  # Documentation-only address; no network is used.


def request(name):
    # Section 6: a new request from an address with a pending one replaces it.
    for pair_id, p in list(pending.items()):
        if p["address"] == address:
            del pending[pair_id]
    pair_id = "fixture-" + str(len(tokens))
    token = "unpredictable-fixture-token-" + pair_id
    tokens[token] = pair_id
    pending[pair_id] = {"address": address, "name": name, "expires": 180}
    notifications.append(pair_id)  # Each new card is emitted exactly once.
    assert len(pending) <= 4
    assert sum(p["address"] == address for p in pending.values()) <= 2
    return token


victim_token = request("legitimate browser")
for i in range(10_000):
    request("another browser behind the same address")

result = {
    "requests": 10_001,
    "pending_at_end": len(pending),
    "max_per_address_observed": 1,
    "notifications": len(notifications),
    "victim_poll_still_pending": tokens[victim_token] in pending,
    "denies_mismatches_expiries": 0,
    "all_defined_concurrency_caps_respected": True,
}
print(json.dumps(result, indent=2))
assert result["victim_poll_still_pending"], "same-address stranger cancels victim without its poll token"
