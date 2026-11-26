- The hub's owner can install plugins from a reviewed community catalog: `agora hub plugins key set FILE` pins the catalog's key,
  `agora hub plugins on` and `agora hub plugins update --url URL` fetch its index, and the hub refuses an index the key did not sign, or
  an older one than it already took under that key. `agora hub plugins install ID` and `upgrade ID` install a plugin only when its
  package is exactly what the signed index lists, after showing what it asks to see (for an upgrade, what changes comes first), and only
  if nothing changed while the owner was asked. Nothing is installed or upgraded on its own. The plugins dialog lists what the catalog
  offers, with the command the owner runs on the hub's machine (ADR-0114).
