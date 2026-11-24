- `agora join` (and Join in the front door) now gives up when its wait to connect runs out, also while the hub's name is still
  being looked up; before, it went on waiting until the name lookup itself gave up, which a slow or unreachable name server can make
  take much longer (#1150).
- The terminal client (`agora tui`), `agora pair open|close|status` and the front door's sign-in now say the hub cannot be reached
  after their 10-second wait to connect, also while the hub's name is still being looked up; before, they waited until the name
  lookup itself gave up (#1150).
- `agora panel` now ends as soon as it says the hub does not answer, also while the hub's name is still being looked up; before, it
  said so after 10 seconds but went on waiting until the name lookup itself gave up (#1150).
