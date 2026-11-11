# Browser language choice (#284 / LAN L19)

Sign-in, Home and Focus offer a labelled native selector with Browser default, English and Türkçe.
The choice applies immediately and is stored only in this browser for this origin. Browser default
removes the override. Other tabs receive updates; an unsupported saved value falls back to the browser.
Unavailable storage still allows changing this visit's language, with a localized notice.

The app subscribes to locale changes without replacing application state. Palette items and existing
sign-in/start/approval error messages follow the selected language. Terminal renderer effects depend
on the terminal source, so translating their ARIA label does not dispose xterm or reconnect its session.

## Verification

`npm test` passes all 83 tests, including explicit preference, browser-default restoration,
storage refusal, document language/direction and subscription removal. Type checking and production
build pass. Existing English fallback and full Turkish message coverage remain checked.

Run the disposable browser proof with a built client, binary and fake-agent fixture:

```sh
PW_DIR=/path/to/playwright TMPDIR=/tmp \
  AGORA_SCREENSHOTS=docs/screenshots/language-choice \
  node scripts/e2e-language-choice.mjs
```

The installed older embedded client fails the same proof at the missing language selector
(`AGORA_TEST_BINARY=/path/to/old/agora AGORA_TEST_EMBEDDED_WEB=1`). The updated client passes:

- Home and Focus preserve the exact xterm DOM instance across language changes. The Home switch
  creates no new WebSocket and retains the selected working-folder input.
- Palette labels update, the preference survives reload, Browser default removes it, and another
  tab's choice is reflected without reloading the first tab.
- Blocking preference writes leaves the current page translated and displays the unsaved notice.
- A separate signed-out browser can choose Turkish before entering a code. An unsubmitted code is
  preserved during the switch; reload remembers the language. Invalid saved preferences are ignored.
- The phone header fits without horizontal scrolling. Screenshots cover both themes and widths.

Language-to-Home-text update with one live shell: **p95 8.6 ms**, 20 alternating choices in Chromium
on Ubuntu (50 ms local render target). This is not a claim about the 500-agent capacity scenario.
Only a disposable shell is launched; no real coding agent, account or live hub state is used.

## Screenshots

| Screen | Dark | Light |
| --- | --- | --- |
| Home, Turkish, desktop | [dark](../screenshots/language-choice/home-tr-desktop-dark.png) | [light](../screenshots/language-choice/home-tr-desktop-light.png) |
| Home, Turkish, phone | [dark](../screenshots/language-choice/home-tr-phone-dark.png) | [light](../screenshots/language-choice/home-tr-phone-light.png) |
| Home, English, desktop | [dark](../screenshots/language-choice/home-en-desktop-dark.png) | [light](../screenshots/language-choice/home-en-desktop-light.png) |
| Sign-in, Turkish, desktop | [dark](../screenshots/language-choice/signin-tr-desktop-dark.png) | [light](../screenshots/language-choice/signin-tr-desktop-light.png) |
| Sign-in, Turkish, phone | [dark](../screenshots/language-choice/signin-tr-phone-dark.png) | [light](../screenshots/language-choice/signin-tr-phone-light.png) |

Themes are set before mounting the terminal so its canvas uses the same tokens as the surrounding UI.
