# Privacy and data handling

Agora is global; the hub may run in the EU, the US, or on a laptop. Design for the strictest regime (GDPR).

- **Data minimization at the hub.** The hub stores coordination data and metadata. Source code, terminal
  output, and credentials stay on nodes unless a client is actively viewing them.
- **Terminal scrollback persistence** is off by default and stored only on the node when enabled.
- **Retention.** Configurable retention for events, audit log, and mesh messages; audit log minimum retention
  can be enforced by policy in team mode.
- **Export and deletion.** Users can export their data and delete their account; deletion removes personal
  data from projections while preserving anonymized audit entries where policy requires.
- **No product telemetry by default.** Any usage analytics are opt-in, documented, and contain no code,
  paths, prompts, or terminal content.
- **Cloud mode.** Region selection for hub data; E2E terminal encryption so the operator cannot read streams.
