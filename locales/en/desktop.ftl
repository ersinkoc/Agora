# The desktop app's own words (apps/desktop): its menu and its notifications. The hub's page has its own (web.ftl).

desktop-menu-help = Help
desktop-menu-safe-mode = Open in Safe Mode

# The app's own notifications (ADR-0048 Amendment A): fixed words with at most an agent's name, checked by the app. The title is
# notify-title (notify.ftl).
desktop-notify-finished-named = { $name } finished.
desktop-notify-needs-you-named = { $name } needs you.
desktop-notify-finished =
    { $count ->
        [one] An agent finished.
       *[other] { $count } agents finished.
    }
desktop-notify-needs-you =
    { $count ->
        [one] An agent needs you.
       *[other] { $count } agents need you.
    }
