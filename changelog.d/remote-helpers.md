- Agents can start helpers on other machines: an agent's `agent_start` with `node` set to another machine raises a card on that machine,
  which you approve there once or with "More like this" (later starts from that machine go ahead without asking, up to 6 at a time, for
  at most 24 hours); the helper shows under its parent in the agent tree. A hub that other people use carries these starts only after
  `agora hub remote-helpers on`, and a machine can refuse them with `mesh.remote_helpers = false` in its policy (ADR-0119).
