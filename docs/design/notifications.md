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
