# Agent accounts on nodes

> **Partly built (ADR-0084).** Built: one manifest field, `[accounts] credential_env`; the node's secret store (`agora node secret set|list|remove`, a
> file per name under `<data>/secrets`, owner-only, on every system; the macOS Keychain is not used); the policy line `credentials.<kind> = NAME, ...`;
> passing the listed secrets in the environment of agents of that kind only, and scrubbing every kind's declared names from the inherited
> environment otherwise; `agora doctor` naming, per kind, whether the node has a credential for it (names only; it does not know whether the login
> works). Still design, not built, everything below this note: login flows run by the node, `status` checks and `agent.not_authenticated`,
> accounts per agent (`config_dir_env`), a window entry for storing a secret (it would pass through the hub), and passing by file descriptor.
>
> **Design, not built (the rest).** Nothing below exists in the code yet: the manifest's `[accounts]` table is parsed but the node never runs a login, never raises `agent.not_authenticated`, and holds no per-account config directory or keychain entry. An agent that is not logged in shows its own login screen in the terminal. See [capabilities](../capabilities.md) and backlog P2-12.

Problem: an agent on a remote VPS needs its own login (subscription or API key). Credentials must stay on the
node (privacy, threat model T7).

Design:
- Each agent kind on each node has zero or more **accounts**, isolated by the agent's config directory
  (`[accounts].config_dir_env` in the manifest, e.g. a dedicated home per account).
- **Login flow:** the user clicks "Log in" in the window; the node runs the manifest's `login` command in a
  terminal streamed to that client (works for browser device-code and paste-code flows). For L2/L3 agents
  that expose authentication methods, the structured flow is used instead.
- **API keys** entered in the window are sent end-to-end to the node and written only to the node's
  OS keychain (Keychain, Credential Manager, Secret Service) or the agent's config; the hub stores only
  `{ kind, node, account_label, authenticated }`.
- **Status checks** run the manifest's `status` command periodically; failures raise `agent.not_authenticated`.
- **Multiple accounts** let users spread work across subscriptions; account choice is per agent start.
