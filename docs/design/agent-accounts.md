# Agent accounts on nodes

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
