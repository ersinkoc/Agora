- Agents can now see every machine of the hub and every agent running on it, in any project: a new mesh tool `machines_list`, and
  `agents_list` with `scope: "hub"`. A hub used by other people shows this only after its owner runs `agora hub fleet on`. With
  `mesh.fleet = false` in a machine's policy file, that machine's agents neither ask for nor see the hub-wide view, and the machine and
  its agents are left out of it; machines that run an older Agora are left out too until they are updated (ADR-0112).
