#!/bin/sh
# Names that differ only in letter case collide on Windows and on a default (case-insensitive) macOS volume, which Linux CI cannot
# see: two tracked files whose paths differ only in case cannot both be checked out there, and a web module import without an
# extension (`./LaunchSettings`) resolves to the other file (`launchSettings.ts` before `LaunchSettings.tsx`), which broke the
# web build on Windows and macOS (comms 2026-11-21, Windows).
fail=0
dups=$(git ls-files | tr 'A-Z' 'a-z' | sort | uniq -d)
if [ -n "$dups" ]; then echo "paths that differ only in case:"; echo "$dups"; fail=1; fi
mods=$(git ls-files apps/web/src apps/web/scripts 2>/dev/null | grep -E '\.(ts|tsx|js|jsx|mjs|cjs)$' \
  | sed -E 's/\.(ts|tsx|js|jsx|mjs|cjs)$//' | tr 'A-Z' 'a-z' | sort | uniq -d)
if [ -n "$mods" ]; then echo "web modules whose import paths differ only in case or extension:"; echo "$mods"; fail=1; fi
exit $fail
