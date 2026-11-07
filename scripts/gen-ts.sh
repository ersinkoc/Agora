#!/usr/bin/env sh
# Generate TypeScript types from schema/ into apps/web/src/gen/ (ADR-0009). No protoc needed.
# Generated files are not committed; run `npm ci` in apps/web first.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/apps/web/src/gen"
mkdir -p "$out"
cd "$root/apps/web"
protos=$(find "$root/schema" -name '*.proto' | sort)
# shellcheck disable=SC2086
npx pbjs -t static-module -w es6 -p "$root/schema" -o "$out/agora.js" $protos
npx pbts -o "$out/agora.d.ts" "$out/agora.js"
