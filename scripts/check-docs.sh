#!/usr/bin/env sh
# Repository hygiene checks that do not need a Rust toolchain.
set -eu
fail=0
# Every ADR listed on disk must appear in the ADR index.
for f in docs/adr/[0-9][0-9][0-9][0-9]-*.md; do
  n=$(basename "$f" | cut -c1-4)
  grep -q "| $n |" docs/adr/README.md || { echo "ADR $n missing from docs/adr/README.md"; fail=1; }
done
# The index says each ADR's status as its file does (architecture review R7: one status, not two that drift apart).
# An Accepted ADR must not still say it is not accepted (a Proposed ADR's warning line left behind when it was accepted).
for f in docs/adr/[0-9][0-9][0-9][0-9]-*.md; do
  if grep -q '^Status: *Accepted' "$f" && grep -q '^Not accepted\. Nothing here may be built' "$f"; then
    echo "$f: Accepted, but still says \"Not accepted. Nothing here may be built\""; fail=1
  fi
done
for f in docs/adr/[0-9][0-9][0-9][0-9]-*.md; do
  n=$(basename "$f" | cut -c1-4)
  file_status=$(sed -n 's/^Status: *\([A-Za-z]*\).*/\1/p' "$f" | head -n 1)
  index_status=$(grep "^| $n |" docs/adr/README.md | head -n 1 | awk -F'|' '{gsub(/ /, "", $(NF-1)); print $(NF-1)}')
  if [ -n "$index_status" ] && [ "$file_status" != "$index_status" ]; then
    echo "ADR $n: docs/adr/README.md says $index_status, $f says $file_status"; fail=1
  fi
done
# A document outside docs/adr that calls an ADR proposed is wrong once that ADR is accepted (dated reviews in docs/reviews are records
# of their day and are not rewritten).
for hit in $(grep -rnoiE "ADR-[0-9]{4}[,:]? *\(?proposed" --include='*.md' docs README.md 2>/dev/null | grep -v -e '^docs/adr/' -e '^docs/reviews/' | tr ' ' '_'); do
  n=$(printf '%s' "$hit" | sed -n 's/.*ADR-\([0-9]\{4\}\).*/\1/p')
  f=$(ls docs/adr/"$n"-*.md 2>/dev/null | head -n 1)
  [ -n "$f" ] || continue
  if grep -q '^Status: *Accepted' "$f"; then
    echo "$(printf '%s' "$hit" | cut -d: -f1-2): calls ADR-$n proposed, but $f is Accepted"; fail=1
  fi
done
# docs/capabilities.md says each ADR's decision as its file does.
for pair in $(grep -oE "ADR-[0-9]{4} (Accepted|Proposed|Superseded|Rejected)" docs/capabilities.md | tr ' ' '='); do
  n=$(printf '%s' "$pair" | cut -c5-8); claimed=${pair#*=}
  f=$(ls docs/adr/"$n"-*.md 2>/dev/null | head -n 1)
  if [ -z "$f" ]; then echo "docs/capabilities.md: ADR-$n does not exist"; fail=1; continue; fi
  actual=$(sed -n 's/^Status: *\([A-Za-z]*\).*/\1/p' "$f" | head -n 1)
  [ "$actual" = "$claimed" ] || { echo "docs/capabilities.md: ADR-$n is $actual, the table says $claimed"; fail=1; }
done
# changelog.d: every fragment is CHANGELOG bullets (lines starting "- ", continuation lines indented), never empty; CHANGELOG.md keeps
# its "## Next" section for the release to fold them into.
for f in changelog.d/*.md; do
  [ -e "$f" ] || continue
  [ "$(basename "$f")" = README.md ] && continue
  if ! grep -q '^- ' "$f"; then echo "$f: no '- ' bullet"; fail=1; fi
  if grep -nvE '^(- |  |$)' "$f" | grep -q .; then echo "$f: a line that is neither a bullet nor an indented continuation"; fail=1; fi
done
grep -q '^## Next$' CHANGELOG.md || { echo "CHANGELOG.md: no '## Next' section"; fail=1; }
# Every Fluent key used must exist in the source locale (placeholder until the i18n lint exists).
[ -f locales/en/main.ftl ] || { echo "locales/en/main.ftl missing"; fail=1; }
# The install scripts check releases against the key they carry: it must be the published release key.
key=$(cat docs/security/release-key.pub)
for f in install.sh install.ps1; do
  grep -qF "'$key'" "$f" || { echo "$f: its release key is not docs/security/release-key.pub"; fail=1; }
done
exit $fail
