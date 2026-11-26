# Settings

- **Account:** profile, passkeys, devices (revoke), language, time format.
- **Machines:** nodes, health, version, update channel, local policy (view; changes require signed approval),
  resource limits, agent accounts per kind.
- **Agents:** installed kinds, integration level per kind, default mode (terminal or chat), registry sources. In the web window today:
  This device → Agents (`#/device/agents`) shows, for one machine at a time, the agents it can start as a grid of cards (installed first,
  then not installed, dashed), how each runs (terminal, chat, chat through an adapter) and the settings and options each takes (opening a
  card shows the values), read only: they are chosen when an agent starts, an option that skips permission questions is marked with the
  red bolt, labelled and never a default, and an offline machine shows what it reported last, marked older. Not built yet: installing from the list (ADR-0087), the "Add an agent" list (ADR-0092), saved defaults per agent.
- **Projects:** default base branch, merge policy, approval policy, retention.
- **Notifications:** per trigger and channel, quiet hours.
- **Appearance:** theme (`themes.md`: a theme for light and one for dark, the theme editor), density, terminal font. In
  the web window today: the title bar's scheme buttons, This device → Appearance, and **Edit theme** in the command palette.
- **Keyboard:** every command and its keys (`keybindings.md`): change, clear, reset, import and export the keybindings file. In the
  web window today: Settings → Keyboard (`#/device/keyboard`), and **Keyboard settings** in the command palette.
- **Advanced:** diagnostics (`agora doctor` download), backups, telemetry opt-in.

## Interface language

The sign-in screen (a quiet select in the sign-in card's footer), Home header and focused agent view offer **Language**: **Browser default**, **English**, **Türkçe**.
The default follows the browser language, falling back to English for unsupported languages. An explicit selection takes effect
immediately and is remembered in this browser for this hub; it also updates other tabs. Returning to **Browser default** removes
the override. This setting is independent of machine/agent languages and is available before signing in.

Changing language preserves open terminals, the hub connection, navigation and unfinished forms. If browser storage is unavailable,
the choice still applies for the current visit and a localized notice explains that it could not be saved.
