#!/usr/bin/env sh
# Repository hygiene checks that do not need a Rust toolchain.
set -eu
fail=0
# Every ADR listed on disk must appear in the ADR index.
for f in docs/adr/[0-9][0-9][0-9][0-9]-*.md; do
  n=$(basename "$f" | cut -c1-4)
  grep -q "| $n |" docs/adr/README.md || { echo "ADR $n missing from docs/adr/README.md"; fail=1; }
done
# Every Fluent key used must exist in the source locale (placeholder until the i18n lint exists).
[ -f locales/en/main.ftl ] || { echo "locales/en/main.ftl missing"; fail=1; }
exit $fail
