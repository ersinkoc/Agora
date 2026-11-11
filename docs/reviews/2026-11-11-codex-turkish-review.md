# Turkish wording review (2026-11-11)

Scope: all six `locales/tr/*.ftl` files, compared with `locales/en/*.ftl`, the error-code specification and the UI vocabulary.
The Codex language review below was followed by the maintainer's explicit human approval on 2026-11-11:
"insan onayı verdim". The human native-speaker review requirement for the Turkish strings revised in #227 is complete.

Forty messages were corrected. Main findings:

- `cli-adapter-exists` called an agent manifest a notification ("bildirim"); it now says "ajan tanımı".
- `tui-usage` used "jeton" while the web client used "token"; both now use "token", and input/output counts are explicit.
- The budget notice now says spending reached a fraction of the budget; the previous sentence lacked a possessive ending.
- The SSH hint now clearly says to copy the hub CA certificate to the remote machine and run enrollment there.
- The grant explanation identifies the permission as what the device key signs, and states that it expires.
- Release-key and CA wording, login failures, resource limits, publisher terminology and several incomplete sentences were clarified.
- Cost strings retain "yaklaşık" once instead of repeating the estimate qualification.

Terminology retained deliberately: "düğüm" for the CLI daemon, "makine" for the UI machine, "hub" where the English source uses it,
"onay" for an approval request and "izin" for permission. Command names and actual shortcut keys (`y`, `n`, `s`, `x`) remain literal.
`main.ftl` needed no wording changes. No message key, interpolation variable, selector or protocol value changed.

Verification: `i18n-lint`; parity of 391 message keys, variables and Fluent selectors against English; all 14 `agora-i18n` tests;
`cargo fmt --check`; `cargo clippy -p agora-i18n --all-targets -- -D warnings`; and repository docs checks.
After generating the schema types with `npm run gen`, the web build and all 77 web tests passed. Turkish gallery screenshots
at desktop and phone widths in both themes are in `docs/screenshots/turkish-review/`.
The Fluent messages with interpolated names avoid suffixes whose pronunciation cannot be known in advance.

The maintainer's approval covers the revised Turkish strings, including the grant and signature messages. No further
human approval is pending for this review.
The English `cli-trusted` message still requests a restart although trusted-device reload is documented elsewhere; correcting that
source behavior description is outside this translation pass.
