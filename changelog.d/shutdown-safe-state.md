- Agents come back after an OS shutdown or reboot more reliably: an agent the shutdown ended is offered to Resume instead of being
  listed as finished, the node writes what it holds first when it is stopped and syncs it to disk, and a node whose event log cannot
  be read sets it aside and starts instead of refusing to. On Windows the node is meant to stop cleanly when the session ends; that
  part is not yet tried on a Windows machine (herdr G11).
