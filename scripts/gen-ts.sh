#!/usr/bin/env sh
# Generate TypeScript types from schema/ into apps/web/src/gen/ (ADR-0009). No protoc needed.
# Generated files are not committed; run `npm ci` in apps/web first.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/apps/web/src/gen"
mkdir -p "$out"
cd "$root/apps/web"
# The schema files are named relative to apps/web, so a checkout path with spaces (common on Windows: C:\Users\First Last) never
# splits them into words. Their own names have no spaces.
protos=$(cd ../../schema && find . -name '*.proto' | sort | sed 's|^\./|../../schema/|')
# shellcheck disable=SC2086
npx pbjs -t static-module -w es6 -p ../../schema -o src/gen/agora.js $protos
npx pbts -o src/gen/agora.d.ts src/gen/agora.js
