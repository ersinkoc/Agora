# Turkish strings: a second reader (2026-11-09)

Status update (2026-11-11): the maintainer has supplied human approval for the revised Turkish strings in #227.
The native-speaker review requirement mentioned below is complete; see [the follow-up review](2026-11-11-codex-turkish-review.md).
The rest of this note records the state on 2026-11-09.

This is **not** the native-speaker review the backlog asks for. It is a mechanical check and one pass by Codex (codex-cli 0.167.0, read-only),
so a person who reads Turkish still has to go through `locales/tr/` before a public release.

**Mechanical check (by the author).** Every key of `locales/en/*.ftl` exists in `locales/tr/*.ftl` and the other way round, and each message
uses exactly the same placeholders (`{ $name }`) in both. No differences.

**Codex pass.** Asked to compare meaning, correctness of the Turkish, terminology and tone, it listed 27 messages. Twenty were changed:

| Key | Change |
|---|---|
| `cli-trusted` | "node'daki" (an untranslated word with an apostrophe suffix) became "bu makinedeki ... düğümü" |
| `cli-hub-notify-refused` | "durumuyla" became "durum koduyla" |
| `protocol-unsupported_generation` | "önerilen" became "sunulan" (what the peer offered) |
| `worktree-unmerged` | rephrased to say the changes are saved nowhere else |
| `agent-kind_unknown` | "kullanılabilir değil" became "kullanılamıyor" |
| `board-unavailable` | "yerelde" became "bu makinede" |
| `mesh-body_too_large`, `mesh-too_many_refs`, `internal` | "başvuru" (an application) became "referans" and "olay numarası" |
| `notify-agent-done` | "bitti" became "işini bitirdi" |
| `palette-placeholder`, `palette-next-attention` | more natural wording |
| `start-limit` | "Makine kapasitesine ulaştı" |
| `signin-intro` | the pronoun "onu" became "hub'ı" |
| `changes-title`, `changes-none` | "commit edilmemiş" became "Git'e kaydedilmemiş" (matching the other files) |
| `history-title` | "{ $name } commit'leri" became "{ $name }: commit'ler" |
| `cli-hub-ssh-bad`, `cli-hub-ssh-install-plan`, `cli-hub-ssh-install-binary` | "yalnızca ... içerebilir", "program dosyası", "Yönetici yetkisi" instead of "ikili" and "root" |

**Left as they are, on purpose.**
- The comma after a placeholder in `agent-mode_unsupported` and `lease-held` (`{ $kind }, { $mode } ...`): the placeholder is a name the text cannot put a
  suffix on, and the comma keeps the sentence readable; a native reader should decide.
- `agent-state-blocked` stays "Girdi bekliyor": Codex wanted "Sizi bekliyor", but the English source says "Needs input", which this matches.
- "Hub" in the web strings: Codex flagged it as internal jargon, but the English source says "your hub" too, so the vocabulary decision belongs to
  the English text and `docs/ux/visual-language.md`, not to the translation.
- The English placeholders in `cli-usage` (`ADDR`, `DIR`, `HOST`, ...): they name command-line arguments and are the same in the English text.

**Still inconsistent (Codex's list, for the person who reviews):** `düğüm` / `node` / `makine` for a node; `ileti` / `mesaj`; `giriş yapmak` / `oturum açmak`;
`onay` / `izin`.
