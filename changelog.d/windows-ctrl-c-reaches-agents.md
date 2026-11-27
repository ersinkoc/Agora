- On Windows, Ctrl-C typed into an agent's terminal now reaches the program running there (a `ping`, a build, a script), also when
  Agora runs as a service: the node's own detached start had switched Ctrl-C off for everything it started (M2.6).
