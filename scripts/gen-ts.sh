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
# The adapter schema is for stdio only. Shared commands still import the enrollment schema.
protos=$(cd ../../schema && find . -name '*.proto' ! -path './agora/v1/adapter.proto' | sort | sed 's|^\./|../../schema/|')
# Keep JSDoc for pbts. The web build strips it.
# shellcheck disable=SC2086
npx pbjs -t static-module -w es6 -p ../../schema --no-verify --no-convert --no-delimited --no-typeurl -o src/gen/agora.js $protos
npx pbts -o src/gen/agora.d.ts src/gen/agora.js
