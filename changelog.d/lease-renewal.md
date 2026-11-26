- On a project shared between machines, a file lease of 30 seconds or more now lasts while its agent runs: the agent's machine
  renews it at half its time (also after the machine restarts, and for a lease it handed over when the project became shared),
  and the lease ends when the agent releases it or stops, or at its time when the machine can no longer reach the hub (ADR-0039).
