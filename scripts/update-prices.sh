#!/usr/bin/env bash
# Rebuilds crates/agora-hub/data/prices.json: the models.dev catalog (https://models.dev/api.json, MIT, see
# THIRD_PARTY.md) cut down to what the hub reads: { provider: { models: { model: { cost: { input, output, cache_read,
# cache_write } } } } }. Models without an input and output price are left out. Usage: scripts/update-prices.sh [api.json]
set -euo pipefail
cd "$(dirname "$0")/.."
src="${1:-}"
if [ -z "$src" ]; then
  src="$(mktemp)"
  curl -fsS --max-time 60 https://models.dev/api.json -o "$src"
fi
python3 - "$src" crates/agora-hub/data/prices.json <<'PY'
import json, sys
root = json.load(open(sys.argv[1]))
out = {}
for pid, p in sorted(root.items()):
    models = {}
    for mid, m in sorted((p.get("models") or {}).items()):
        c = m.get("cost") or {}
        if not isinstance(c.get("input"), (int, float)) or not isinstance(c.get("output"), (int, float)):
            continue
        models[mid] = {"cost": {k: c[k] for k in ("input", "output", "cache_read", "cache_write") if isinstance(c.get(k), (int, float))}}
    if models:
        out[pid] = {"models": models}
json.dump(out, open(sys.argv[2], "w"), separators=(",", ":"), sort_keys=True)
print(sum(len(v["models"]) for v in out.values()), "models priced")
PY
