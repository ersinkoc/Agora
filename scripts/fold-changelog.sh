#!/usr/bin/env bash
# Folds changelog.d/*.md into the top of "## Next" in CHANGELOG.md, newest first (by the commit that added each file), and removes
# the fragments with `git rm`. Run by the release pull request before "## Next" becomes "## X.Y.Z".
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
frags=()
while IFS= read -r f; do frags+=("$f"); done < <(
  for f in changelog.d/*.md; do
    [ "$(basename "$f")" = README.md ] && continue
    [ -e "$f" ] || continue
    # Seconds of the commit that added the file (0 when not committed yet), then the name, so the order is stable.
    t=$(git log --diff-filter=A --format=%ct -1 -- "$f" 2>/dev/null || true)
    printf '%s\t%s\n' "${t:-0}" "$f"
  done | sort -t$'\t' -k1,1nr -k2,2 | cut -f2
)
[ "${#frags[@]}" -gt 0 ] || { echo "changelog.d: nothing to fold"; exit 0; }
grep -q '^## Next$' CHANGELOG.md || { echo "CHANGELOG.md has no '## Next'"; exit 1; }
block=$(mktemp)
for f in "${frags[@]}"; do sed -e '$a\' "$f" >> "$block"; done
# Insert after the "## Next" line and its blank line; drop a "_Nothing yet._" placeholder.
awk -v block="$block" '
  done_ins == 0 && /^## Next$/ { print; getline; print; while ((getline line < block) > 0) print line; done_ins = 1; next }
  done_ins == 1 && /^_Nothing yet\._$/ { next }
  { print }
' CHANGELOG.md > CHANGELOG.md.new
mv CHANGELOG.md.new CHANGELOG.md
rm -f "$block"
git rm -q "${frags[@]}"
echo "folded ${#frags[@]} fragment(s) into CHANGELOG.md"
