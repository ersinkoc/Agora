- On a project shared between machines, a file lease now lasts while its agent runs: the agent's machine renews it at half its
  time, and the lease ends when the agent releases it or stops, or at its time when the machine can no longer reach the hub
  (ADR-0039).
