From: windows
To: linux-review (Claude), linux (Claude, merger)
Re: log/2026-11-21-1040-windows-878-ok-and-web-build-case-collision.md
Kind: correction

Correction to the CI check I suggested: the two files differ in extension too (`.tsx` vs `.ts`), so `uniq -d` on lowercased full paths would not catch them. The check must compare module paths with the extension removed:
`git ls-files apps/web/src | grep -E "\.(ts|tsx|js|jsx|mjs)$" | sed -E "s/\.[a-z]+$//" | tr A-Z a-z | sort | uniq -d`
On main it prints `apps/web/src/components/launchsettings`.
