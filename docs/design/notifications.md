# Notifications

Pipeline: event → rule → channel → delivery.

- **Triggers:** agent `blocked`, agent `done`, approval requested, task completed, node offline, budget
  threshold, update available.
- **Channels:** in-app toast and badge; desktop OS notification (Tauri); web push; mobile push (later); email;
  webhook (Slack/Discord/Teams through webhooks first).
- **Per-user preferences:** per trigger and channel, quiet hours, "only when I am not looking at it".
- **Dedupe and batching:** collapse repeated states; digest mode for low-priority events.
- **Localization:** notification text rendered in the recipient's locale from Fluent keys.
- Approval notifications include deep links; acting on them still requires a signed approval (ADR-0020).

## Implementation status
`agora-hub::notify` implements the rule step: per-trigger channel preferences, quiet hours (wrapping midnight; only the in-app badge
stays on), "only when I am not looking at it", dedupe over 60 s, and a digest for low-priority triggers. Text is rendered from
`locales/en/notify.ftl`. Delivery: `agora-hub::deliver` sends to `ntfy` topics and `webhook` URLs (Slack and Discord take the same JSON post) over
plain HTTP/1.1 with rustls for `https` (system root certificates), 10 s timeouts, one short request per notice. `agora-hub::notifier`
watches the hub's live events (an approval waits, an agent is blocked or done, a machine went offline), routes them through the
rules above and sends from a thread of its own. Targets are listed in `<hub dir>/notify`, one `ntfy URL` or `webhook URL` per line;
`hub run` reads it at start. Not yet: desktop notifications from the hub, web push, email, Teams-style webhooks, the digest
delivery, stored per-user preferences, the settings screen, (`agora hub notify --ntfy URL | --webhook URL` adds a target, `--test` sends a test message to each, no option lists them).
