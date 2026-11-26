- Starting the node a second time on a machine where it already runs (or a second node on the same data folder) now stops at once,
  before it reads or writes anything: it no longer brings back an agent the running node offers to resume, leaving a copy of it
  running that no node shows, and no longer writes into the running node's records (audit F2040)
- After the hub lost part of what a machine reported (a long time offline, a hub restored from a backup, a machine back with an
  empty data folder), the window again shows which of your devices that machine trusts, instead of an old list (audit F2041)
