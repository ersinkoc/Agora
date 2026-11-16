"""Executable contract traces, NOT a test of an implemented pairing server.

Run: python3 docs/reviews/0632-revised-state-traces.py
The literal and proposed interpretations expose the two lifecycle ambiguities.
All inputs are synthetic; no accounts, listeners or user credentials are used.
"""
import json


def browser_delivery(invalidate_on_approval):
    token_hash_valid = True
    state = "Pending"
    events = ["create(t=0)", "poll(t=2): pending", "approve(t=3)"]
    state = "ApprovedAwaitingDelivery"
    if invalidate_on_approval:
        token_hash_valid = False
    delivered = 0
    for now in (4, 6):
        if now < 180 and token_hash_valid and state == "ApprovedAwaitingDelivery":
            delivered += 1
            state = "Consumed"
            token_hash_valid = False
            events.append(f"poll(t={now}): cookie delivered")
        else:
            events.append(f"poll(t={now}): token invalid")
    return {"events": events, "cookie_deliveries": delivered}


def machine_wait(apply_idle_timeout_to_pending):
    # The reveal was received at t=0; no pairing message is needed while the
    # owner compares the number. No specified heartbeat is assumed.
    closes_at = min(30, 300) if apply_idle_timeout_to_pending else 300
    return {"reveal": 0, "approve": 45, "closes_at": closes_at,
            "approve_at_45_succeeds": 45 < closes_at}


def browser_churn():
    # Same-IP, zero-time churn: all attempts count before allocation. The
    # attacker never has the victim's capability. Numeric rates are section 6.
    pending = {"victim": "victim-token-hash"}
    attempts = 1
    notices = 1
    admissions = 1
    for n in range(10_000):
        attempts += 1
        if attempts > 3 or len(pending) >= 2:
            continue
        pending[f"attacker-{n}"] = "different-token-hash"
        admissions += 1
        # Hub-wide one-minute notification cooldown is still active.
    return {"attempts": attempts, "admissions": admissions,
            "pending": len(pending), "victim_token_unchanged":
            pending.get("victim") == "victim-token-hash", "notices": notices}


observed = {
    "scope": "contract models, not implementation tests or cryptographic proof",
    "head": "b2281e204aa3bb76ec3852140c5a37c8015372ed",
    "literal_invalidate_at_approval": browser_delivery(True),
    "proposed_consume_at_delivery": browser_delivery(False),
    "literal_30_second_idle": machine_wait(True),
    "proposed_pending_deadline": machine_wait(False),
    "revised_same_address_churn": browser_churn(),
}
assert observed["literal_invalidate_at_approval"]["cookie_deliveries"] == 0
assert observed["proposed_consume_at_delivery"]["cookie_deliveries"] == 1
assert not observed["literal_30_second_idle"]["approve_at_45_succeeds"]
assert observed["proposed_pending_deadline"]["approve_at_45_succeeds"]
assert observed["revised_same_address_churn"] == {
    "attempts": 10001, "admissions": 2, "pending": 2,
    "victim_token_unchanged": True, "notices": 1,
}
print(json.dumps(observed, indent=2))
