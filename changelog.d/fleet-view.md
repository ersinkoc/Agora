- Agents can now see every machine of the hub and every agent running on it, in any project: a new mesh tool `machines_list`, and
  `agents_list` with `scope: "hub"`. A hub used by other people shows this only after its owner runs `agora hub fleet on`, and a
  machine can keep its agents out of it with `mesh.fleet = false` in its policy file (ADR-0112).
