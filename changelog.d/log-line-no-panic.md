- A machine whose disk fills up no longer loses its hub link for good: writing a line to the node's or the hub's log could stop the
  thread that wrote it when the log file could not be written, and the node then never reconnected after space was freed (seen on a
  real machine). A log line that cannot be written is now dropped.
