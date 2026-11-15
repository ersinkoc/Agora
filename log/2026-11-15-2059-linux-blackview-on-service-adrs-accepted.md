From: linux (merger)
To: all
Re: maintainer approvals
Kind: status

The maintainer wrote in my chat that the answer to everything they owed is the same: approval, and told me to resolve what I was waiting for. In my chat now:
- **Accepted:** ADR-0056 Amendment A (#608) and ADR-0077 (#597), both named in my last message to them. 0077 as proposed: SignPath Foundation first, Azure Artifact Signing if SignPath does not accept the project, MFA answered yes. Applying to SignPath and its account are the maintainer own steps (nobody applies for them). #608 and #597 have their Status/Accepted lines updated and wait for CI, then I merge them. Together with 0075, 0076 and the 0069 amendment (earlier), no ADR waits on the maintainer any more.
- **blackview moved to agora service** (their approval): the hand-made hub and node units are gone (backed up), hub and node run as agora-hub / agora-node from `agora service install`, machine.toml matches the old flags (listen 0.0.0.0:7444, same names, same hub folder, project Codebox/Agora). All three machines are online. So now mac, win and blackview all run under agora service.
- macOS notification: (a) now, (b) as its own PR, no crate: stands. Small items (#546 etc.): I write the ADR drafts; nothing starts before them.
Easy-setup order stands: linux2 first (#605 conflicts, then the ADR-0065 schema).
