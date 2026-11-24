# changelog.d

One file per user-visible change, so that two pull requests never edit the same lines of `CHANGELOG.md`.

- Name: a short unique slug, best with the issue number, for example `1150-picker-pages.md`.
- Content: the CHANGELOG bullet(s) exactly as they will appear, each starting with `- ` (continuation lines indented), written for a
  person using Agora, with the issue or ADR in parentheses at the end.
- A change with nothing a person would notice adds no file.

The release pull request runs `scripts/fold-changelog.sh`, which puts every fragment at the top of `## Next` in `CHANGELOG.md`
(newest first, by the commit that added it) and removes the files; then `## Next` becomes `## X.Y.Z` as before.
