# Settings

- **Account:** profile, passkeys, devices (revoke), language, time format.
- **Machines:** nodes, health, version, update channel, local policy (view; changes require signed approval),
  resource limits, agent accounts per kind.
- **Agents:** installed kinds, integration level per kind, default mode (terminal or chat), registry sources.
- **Projects:** default base branch, merge policy, approval policy, retention.
- **Notifications:** per trigger and channel, quiet hours.
- **Appearance:** theme (`themes.md`: a theme for light and one for dark, the theme editor), density, terminal font. In
  the web window today: the title bar's scheme buttons, This device → Appearance, and **Edit theme** in the command palette.
- **Advanced:** diagnostics (`agora doctor` download), backups, telemetry opt-in.

## Interface language

The sign-in screen, Home header and focused agent view offer **Language**: **Browser default**, **English**, **Türkçe**.
The default follows the browser language, falling back to English for unsupported languages. An explicit selection takes effect
immediately and is remembered in this browser for this hub; it also updates other tabs. Returning to **Browser default** removes
the override. This setting is independent of machine/agent languages and is available before signing in.

Changing language preserves open terminals, the hub connection, navigation and unfinished forms. If browser storage is unavailable,
the choice still applies for the current visit and a localized notice explains that it could not be saved.
